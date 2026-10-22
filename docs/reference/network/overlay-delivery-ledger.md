# Overlay delivery gate ledger

## Purpose and scope

This page is the run history of the live overlay delivery gate in
`.github/workflows/live-tests.yml`: eight independent scanner samples per
run, pinned to a four-vCPU runner, with a 600 s delivery budget per sample
(`TON_MEMPOOL_LIVE_TIMEOUT_SECS`) and a pass threshold over the sample set.
Each record names the run id, the commit under test, the budget, the
outcome, and the counters that separate the arms of the gate - admission
(`unique_pingers`, `dht_pings`, `membership_answers`), transport health
(`pushers`, `custom_messages`, `ADNL late answer`, the demux heartbeat),
and publication (`stored on N nodes`, `served_by_dht`). The matching
transport rules live in [ADNL UDP](adnl-udp.md) and
[Overlay protocol design](overlay.md).

## The two-tier overlay ecosystem

Mainnet mempool overlays are not one uniform population. Live scanner runs
show two classes of peers with different admission behaviour, and the split
lines up with the implementation each peer runs:

- **Upstream full nodes** (`ton-blockchain/ton`) follow the admission chain
  above end to end. They call `add_peers(query.peers_)` on inbound
  `overlay.getRandomPeers`, drain the pending set, originate `overlay.ping`,
  and after the pong mark the node verified. Every live peer that pinged this
  scanner was of this class, and every such peer also echoed
  `recv_addr_list_version` - the marker that it stores a routable
  `adnl.addressList` for us (`AdnlPeerPairImpl::send_packet`).
- **pytoniq-based bots** (the mempool processer fleet, e.g.
  `yungwine/ton-mempool` with `pytoniq.adnl.OverlayManager`) answer queries
  but never echo `recv_addr_list_version` and never originate `overlay.ping`.
  Their membership code (`OverlayManager`, `ShardOverlay`) is a private
  pytoniq extension absent from every public pytoniq release, so its inbound
  `query.peers` handling is not observable; live evidence shows the bots do
  not admit a fresh identity from queries. Bots bootstrap their member lists
  from the DHT instead: `DhtClient.get_overlay_nodes` reads the
  `overlay.<id>.nodes` value (`pytoniq/adnl/dht.py`), and the bot processer
  wires that client into `OverlayManager`.

The DHT announce path is therefore the bot fleet's root of trust, and it is
closed to an ed25519 scanner identity: upstream `dht.updateRule.overlayNodes`
`check_value` (`dht/dht-types.cpp`) requires the value's keyDescription id to
be a `pub.overlay` key, and `pub.overlay` keys are named blobs without
signing material (`keys/keys.hpp` `pubkeys::Overlay`, `EncryptorOverlay`) -
only full nodes hold the paired private form. pytoniq itself cannot store
such values either (`DhtClient.store_value` rejects the `overlayNodes` rule
with `DhtError('currently overlay is not supported')`). This scanner reads
the value for discovery (`lookup.rs`) but cannot and must not write it.

The practical consequence is a bimodal delivery distribution, but the
observed mechanism is narrower than a listing cascade. A bot could in
principle learn this node when a full node samples it into a
`getRandomPeers` answer (`4 / |peers_|` per answer) and then resolve its
DHT `address` value (`DhtClient.get_overlay_node`), but in every recorded
run - the delivering ones included - publishing ended with
`dht address publish: stored on 0 of 4`, so that value never existed, no bot
could resolve it, and the cascade remains a possible path with no positive
live evidence. What the logs show instead is the direct path: a growth or
bootstrap session peer answers a query, originates `overlay.ping`, marks
this node verified after the pong, and pushes broadcasts to it. The
recorded arms: run 38594880666 (2026-10-14, `386042b`) delivered ~80 s into
the wait, seconds after a growth-phase peer pinged it and started pushing
`overlay.message` custom datagrams; run 38558546040 (2026-10-14, `4894e5f`)
delivered at ~295 s after 4313 such datagrams; runs 38579887480 (2026-10-14,
`5fec546`), 38723511984 (2026-10-15, `694aa49`), and 38745254834 (2026-10-15,
`4274f0e`) received none within the full 300 s budget despite normal
membership and pinger counters. The earlier 129-192 s band is superseded by
these arms: timing follows whether any neighbour holder samples this node in
its fan-out, and `dht_pings_received` was 0 in every run while
`membership_listed` stayed 0 until run 38828796572 recorded 4 listings from
one peer; neither counter tracks the push path. A delivery's absence in a
300 s window still proves nothing about this node's reachability.

Roster freeze: every recent run saturated at `registered_peers=65-71` against
the default `overlay_max_peers=64`, so the managed roster froze near the cap
within the first minute, all admission tickets were spent early, and growth
stopped adding fresh full-node contacts. The CI test now runs with
`TON_MEMPOOL_LIVE_OVERLAY_MAX_PEERS=256`, `MempoolConfig.growth_interval=3s`,
`churn_interval=3s`, and `quality_eviction(true)` with a pre-wait burst of
four growth rounds, so eviction -> growth round -> new admission ticket
cycles continue throughout the 300 s wait.

Run 38819591750 (2026-10-15, `fb9afc9`, 300 s budget): timeout arm again.
Aggressive growth (`max_peers=256`, `growth_interval=3s`, `churn_interval=3s`,
pre-wait burst of 4 rounds) raised metrics: `registered_peers=94`,
`address_storing_peers=145`, `unique_pingers=9`, `membership_answers=2400`,
`random_peers_queries_sent=3226`. Despite the strongest admission window
yet recorded, `custom_messages=0` and `overlay_packets=0` — no full-node
neighbour holder sampled this node into its `get_neighbours(propagate_broadcast_to() = 5)` fan-out. This confirms the push lottery is the bottleneck:
even with ~9 verified neighbours, the per-wave push probability (~5/|neighbours|)
can yield zero inbound broadcasts for an entire 300 s window.

Run 38828796572 (2026-10-16, `7ae1cbd`, 300 s budget): delivered. The delta
from run 38819591750 is documentation text only - the delivery test code is
identical - and the admission metrics land in the same band
(`unique_pingers=9`, `address_storing_peers=125`, `membership_answers=2724`,
`random_peers_queries_sent=2436`; the first recorded nonzero listing,
`membership_listed=4` from one peer). This arm counted `custom_messages=1`
and delivered at `time_to_message=222.5s`, while the earlier arm counted zero
for its entire window. The pair - red 22:48 UTC, green 00:19 UTC, same code -
is the cleanest recorded evidence that a live delivery outcome is a per-run
result of the upstream neighbour draw and push fan-out above, not a
client-state difference.

Run 38865936964 (2026-10-16, `cbf6b4a`, 300 s budget): timeout arm again
(`custom_messages=0`, `overlay_packets=0`, `registered_peers=89`,
`unique_pingers=10`, `address_storing_peers=131`, `membership_answers=2522`,
`random_peers_queries_sent=3230`) - the third hardened timeout arm, again
zero inbound broadcasts at healthy admission counters. The CI gate therefore
stopped sampling once: the `live-delivery` job now runs
`TON_MEMPOOL_LIVE_DELIVERY_SAMPLES` (default 8) independent samples in
parallel and passes when at least one delivers. With the recorded per-window
rate (3 green of 9 arms, `p ~ 1/3`) a red gate is a `(1 - p)^8 <= ~4%` false
alarm and therefore a real regression signal, while a single-sample gate was
red on about two healthy runs of three.

Run 38909762774 (2026-10-16, `fe0fa8b`, 300 s budget): delivered and failed a
later phase.  Sample 1 delivered at `time_to_message=106.9s`, then died in the
post-delivery non-seed roundtrip check: all six registry members that the phase
drew failed to answer while the manager held live sessions to other members, so
the phase now probes in waves (connected members first, up to three waves of
six, stopping at the first answer).  The gate is the first where every sample
admitted pingers (`unique_pingers=7-14`, `pings_received=26-61`, against the
previous 0-7 band) and the first with reproducible `served_by_dht=true`
(samples 3, 5, 7: `stored on 3 / 3 / 2 node(s)`), so the admission funnel and
the publish path moved while the push lottery kept seven samples at
`custom_messages=0`.

Run 38912409135 (2026-10-16, `b19ce81`, 300 s budget): all eight samples timed
out with `custom_messages=0`, `unique_pingers=5-9`, `pings_received=14-35`,
`address_storing_peers=99-137` and `membership_listing_peers=0`.  The publish
stage, not admission, was the weak arm in this gate: five samples ended at
`no externally reachable address` and one at `stored on 0 node(s)`; only the
remaining two stored and verified.  The STUN agreement check was counting one
observation per hostname and the two Google hostnames resolve to the same
server, so two answers from a single NAT destination could pass as agreement.
A local probe of the public STUN fleet showed Cloudflare, miwifi, sipnet and
blackberry all answering from the same public IP with a *different* mapped
port per server - the signature of a per-destination (symmetric) NAT, whose
mapping is only valid toward the server that observed it.  Agreement now
counts distinct resolved server addresses only, any disagreement from a
distinct destination disqualifies the round, servers that stayed silent are
retried in a second sweep over a wider list, DNS resolution is bounded, a
store round that stored nothing is retried once, and the gate job prints the
per-sample STUN observations into the job log.

Run 38917780756 (2026-10-16, `1a5cf52`, 300 s budget): green, and the first
gate that classified the runner's NAT.  All eight samples published a
`dht.address` value: Cloudflare (`162.159.207.0:3478`) and Google
(`74.125.250.129:19302`) - two distinct destinations - reported the same
mapped port for each sample's shared socket (one port per sample, 5184-5191),
so the runner's mapping is endpoint independent and every sample passed the
distinct-destination agreement rule.  Sample 7 delivered at
`time_to_message=274.8s` and satisfied the gate; samples 1/3/5/6/8 were
stopped once the winner was found.  The two timed-out samples still recorded
healthy admission (`unique_pingers=4/6`, `pings_received=16/20`,
`membership_answers=1174/1240`), and sample 4 recorded the first non-zero
`membership_listing_peers=2`: full nodes now resolve the published address,
admit this node, and list it back in their own `getRandomPeers` answers.

Run 38922689533 (2026-10-17, `f5f4d69`, 300 s budget): green again.  Sample 3
delivered at `time_to_message=269.9s`; all eight samples resolved an
endpoint-independent mapping (Cloudflare plus Google or miwifi reporting the
same port per sample, 17408-17415).  The publish store round split four/four:
samples 2/4/5/7 stored on 2-4 nodes and verified (`served_by_dht=true`), while
samples 1/3/6/8 recorded six `findNode` failures and two enrichment
exhaustions and ended at `stored on 0 node(s)` even after the retry.  The
delivered sample 3 is one of the unpublished ones, so the window's delivery
came from the direct-announce admission path, not from the stored value; the
timed-out samples kept healthy admission (`unique_pingers=2-9`,
`pings_received=7-34`, `membership_answers=1097-1378`).  Both post-fix gates
delivered at `~270s` of the 300 s budget.

Run 39005547863 (2026-10-17, `5861a89`, 300 s budget): red, all eight samples
timed out at the strongest admission counters recorded so far - sample 7 alone
answered `pings_received=64` from `unique_pingers=16` (pongs = pings in every
sample, `custom_messages=0`), sample 6 reached 12 pingers, three samples
recorded `membership_listing_peers=1`, and every sample resolved an
endpoint-independent mapping.  Publication split: samples 7/8 stored on 3
nodes and verified (`served_by_dht=true`; the sample with the most admitted
holders was published and still saw no broadcast), samples 2/3 stored on 2
without a serving responsible node, and samples 1/4/5/6 hit six `findNode`
failures with two enrichment exhaustions and stored nothing even after the
retry.  Nothing in the window's own counters separates this red draw from the
two green ones: the remaining failure mode is the upstream neighbour-slot
lottery - `process_pending_peers_rate_limiter_{60.0, 60}` services one pending
peer per second in random order from a queue capped at 100, and
`update_neighbours(2)` swaps two neighbours plus five plumtree neighbours
every `U(30,120)` s from up to 100 random draws over the holder's peer table -
and no client-side lever reaches it (announce cadence, reach, publish
correctness, and the pong path are all verified working in this run).

Follow-up policy (2026-10-17): the per-sample delivery window is now 600 s
(the `live-delivery` workflow fallback; k=8 unchanged, the
`TON_MEMPOOL_LIVE_TIMEOUT_SECS` repo-variable override is kept).  The
decision followed this red draw: with no client-side lever left against the
upstream neighbour-slot lottery, a longer window raises the per-sample
probability without the CPU and DHT contention a larger sample count would
add - at the recorded ~8 % per 300 s the constant-hazard extrapolation is
~15 % at 600 s, and the k=8 gate green rate moves from ~35-50 % to ~74 %.

Run 39007384341 (2026-10-17, `a018ab3`, 600 s budget): red, and the wider
window paid off on its first run - sample 4 delivered at
`time_to_message=332.7s`, past the old 300 s deadline, then failed the
post-delivery non-seed roundtrip phase with all 18 probes (3 waves of 6)
unanswered.  The other seven samples timed out at the strongest admission
counters recorded so far (sample 2 answered `pings_received=136` from
`unique_pingers=30`, sample 1 `114`/`18`, sample 8 `126`/`14`), and every
sample resolved an endpoint-independent mapping again.  The roundtrip phase
turned out to be the only dial in the tree that opened a second socket with
a foreign keypair: every other path - manager sessions, DHT lookups, address
resolution, growth - resolves through `AdnlUdpTransport::for_node` and
shares the node's single source address and identity, the rule the transport
documents so a peer never stores a dead one-shot port.  Registry members
answer that identity and almost every foreign one goes silent, which is the
0-of-18 outcome.  The phase now probes through the same construction
(`for_node` + `session_for_peer`, so a member with a live session answers its
probe on it): the member's recorded endpoint must carry traffic for this
node's identity, and the endpoint-staleness lottery of the wave ladder is
all that remains.

Run 39008789830 (2026-10-17, `3a791ac`, the phase fix): red on the pure
delivery lottery - all 8 samples timed out at 600 s with healthy counters
(max `unique_pingers=17`, `pings_received=75`; all 8 resolved an
endpoint-independent mapping; three samples recorded
`membership_listing_peers=1`), so the phase fix was not exercised by the
draw.  The classified re-run of the same job then died inside the
classification itself: the runner executes the step with `bash -e`, and a
`field="$(grep ... | ...)"` extraction whose grep finds no match aborts the
whole step, which a split metrics line (interleaved writes from the test's
threads) triggers - header printed, no diagnosis, exit 1.  The script now
disables errexit before any diagnostics, keeps a degraded prefix when the
structured census line cannot be parsed, and uploads the raw per-sample
logs as an artifact on red draws; the failure was reproduced against a
split-line fixture before the fix and cleared after it.  Sixteen 600 s
samples have now delivered once (332.7 s), and the admission counters show
why the rate moves per draw (30 pingers on one gate, 17 on the next), so
the ~15 %/sample window estimate stays an upper bound until more draws
accumulate.

Run 39011060928 (2026-10-17, `aec38f6`, 600 s budget): red - 2 of 8 samples
delivered (the best at `time_to_message=360.2s`), and the delivered sample
failed the post-delivery non-seed UDP roundtrip phase with 0 of 18 probes
(panic at `live.rs:612`).  The phase failure was neither endpoint staleness
nor member-side: the identity fix from 39007384341 dialed through `for_node` +
`session_for_peer`, and `session_for_peer` adopts the requested address on the
shared session unconditionally.  A growth round or probe wave fires all of a
member's recorded endpoints concurrently, so whichever candidate adopted last
- a dead recorded port, five of six in the captured member's list - became
the session's single send address; the manager's keepalives, growth queries,
and every later probe were then addressed to it while the member listened on
its live port, and only queries sent near a re-adopt of the live port were
answered.  The captured member's shape fits exactly: 51 answers spread over
the run, none for a growth query that followed a dead-port adopt, and a lone
exchange to the live port under the same manager-like keepalive stream
answered in 73 ms.  A local replica of the sweep (six concurrent probes plus
the keepalive stream) reproduced the black-hole deterministically: all six
probes timed out and every subsequent keepalive went to the dead port for the
rest of the run.  The fix separates dialing from ownership: a session's
address changes only through a deliberate adopt (`session_for_peer`, used by
the manager, bootstrap, and reconnect paths), dial-only callers get the same
session through `session_for_probe` without repointing it, and probes that
dial one endpoint carry it on the query itself
(`overlay_get_random_peers_to`, `send_overlay_get_random_peers_to`).  The
growth path and the phase now use both, the phase's `for_node` identity
construction is unchanged, and the offline regression test
`tonutils_adnl::adnl::udp::tests::probe_sessions_keep_the_adopted_address`
pins the contract.

Run 39027490505 (2026-10-17, `7fcdbaf`, 600 s budget): red - 2 of 8 samples
delivered (the best at `time_to_message=24.9s`), and both delivered samples
failed the post-delivery non-seed roundtrip phase with 0 of 18 probes
(panic at `live.rs:622`).  Every probe failed in the inner wait
(`overlay getRandomPeers timed out after 10s`): the query was sent and no
answer arrived within the window.  The captured sample closes every
whole-path theory:

- The probe destination equals the keepalive destination.  Each of the 18
  probed targets has exactly one adopted address across every
  `overlay UDP session established` line of the run (the duplicate connects
  of four targets repeat the same address), it is the address the registry
  holds and the phase printed, and the live test's `native_udp(..., None)`
  keeps the factory on plain `connect_for_overlay`, so no silent
  `with_channel` adoption exists.  Session, channel state, and packet shape
  of a probe and a keepalive are identical.
- The recurring `createChannel` clusters (`channel requested` every
  ~10-20 s) are the six configured seeds, not overlay members: each cluster
  repeats one per-seed channel key and never confirms.  The seeds are the
  growth fallback's DHT nodes (`query_overlay_seed`), so the loops are
  retry noise and non-causal for the phase.
- No transport or session stall spans the phase.  Inbound-answer diags, the
  address-view diags, and `overlay getRandomPeers answer` lines keep their
  steady per-bucket density through every wave window (answer diags 19-49
  per 250 lines across lines 7500-9500) while members answer hundreds of
  queries, and the sample carries exactly one inbound custom datagram, so
  no ingest or demultiplexer pressure exists either.
- The probe rides the manager's warm session: `session_for_probe` upgrades
  the same `SessionInner` (`session_for_inner` with `adopt = false`), no
  fresh `channel requested` line appears for any probed target at wave
  time, and registration precedes the send (`register_pending` before
  `send_overlay_get_random_peers_with_id`), so neither a second session nor
  a registration race can explain the timeout.

The same members answer fire-and-forget `overlay.getRandomPeers` queries
through the same windows while every probed query id goes unanswered, and a
captured log cannot yet say whether the members answer the probed ids at
all: the receive side logs every answer with its query id, but the send
side never logged the ids it sends.  `send_overlay_get_random_peers_with_id`
now logs peer, query id, effective destination, origin (`explicit` probe
versus `session` address), channel count, and whether a `createChannel`
rode along, and the three previously silent channel-control branches
(repeated `createChannel`, `confirmChannel` without a pending request or
with the negotiated key) log at debug, so the next classified gate's
query-id join can attribute each probe answer (answered, answered late,
never answered) against the keepalive control stream.

Run 39032749928 (2026-10-17, `5b14291`, 600 s budget): red, and the send-side
qid log answered the previous record's open question - the members do answer
the probed ids, only ~27 s late.  The sample captured every wave-1 probe sent
(sends 25576-25581) and answered at 27411-27445, ~1833-1864 log lines later;
all 25 answered probe pairs land in that band, and the join over the whole
sample (all 2091 answer/send matches among 3766 sends and 2752 answers) shows
the send-to-answer delta growing from 6-32 lines (sub-second, sends around
lines 4500-4650) to a ~1800-2100-line plateau by late run.  The lag is global
inbound, not per-member: `recv_v=Some` members and `recv_v=None` members
answer equally late (the Some-echo target carries 67 `address view mismatch`
diags and still answers ~1833 lines late), every peer's delta grows in
lockstep, and member-side theories - nochannel rate limiting, address-list
loss, channel churn - are non-causal for the delay.

The mechanism was in this transport.  `AdnlUdpTransport::run` delivered each
datagram with `route.queue.send(packet).await` into a bounded (64) session
queue; one session whose consumer stops draining fills its queue and inbox
(64 + 64 datagrams), and the awaited send then wedges the single shared
receive task: every other session's datagrams stay in the kernel socket
buffer and arrive only as fast as the stalled session's consumer drains.
That matches the observed shape - sends prompt (each send logs after its
socket write returns), answers late by an amount that grows with the run's
session and subscriber count and plateaus when the socket buffer is full,
uniformly across all peers, and a handful of local sessions never
reproducing it.  The demux now `try_send`s and drops on a full queue, so the
backlog and the loss stay confined to the slow session like any other UDP
loss; the regression test
`tonutils_adnl::adnl::udp::tests::saturated_session_does_not_stall_the_shared_demux`
reproduces the stall on the old code (a sibling session starved past 5 s
while an undrained session's queue is full) and passes with the fix.

Run 39105081595 (2026-10-18, `3cb0e8e`, 600 s budget): red - 7 of 8 samples
timed out with no inbound broadcast, and the delivered sample (at
`time_to_message=303.788s`) failed the phase 0 of 18 with every probe timing
out after 10 s.  The demux fix did not engage: zero `queue full` drops in
all eight samples.  The signature is not the previous run's stall.  The
probed members answered other queries during the phase window - 117 answers
landed after the phase start, all for sends from lines 9130 to 14791 - yet
nothing asked inside the window came back inside it, and the whole-sample
join (1290 answered pairs among 3043 sends) shows the send-to-answer gap
growing from a ~283-line median for the first 2000 lines to a ~3300-line
plateau, roughly double the previous run's plateau.  The growth is uniform:
every peer with at least six matched answers has a higher second-half than
first-half median delta (75 of 75, median drift +1846 lines), so no peer is
fast while others are slow.  Channel state does not discriminate either:
channel-backed sends (`channels=1 control=false`) delay like channel-less
ones, and only 12 of 1073 channel-backed answers arrived in under 100 lines.
Late answers for timed-out probes were invisible in this artifact - the
waiter is already gone and `log_stray_answers` covers only queries the
session never sent - so the follow-up commit adds answer-age logs (delivered
and late), a demux heartbeat (datagrams/sent/dropped/unrouted per 10 s with
real wall-clock dt), and Linux `/proc/net/udp` socket stats (queued bytes
and kernel overflow drops) to separate "this process read the answer late"
from "the answer arrived late".

Run 39107770528 (2026-10-18, `0990ae3`, 600 s budget): red with the same
no-inbound-broadcast arm in all eight samples (`membership_answers` of
1668-2127 while `pushers`, `dht_pings`, and `custom_messages` stayed zero),
and the heartbeat the previous record added puts the delay in this crate's
receive path.  Beat gaps run 10-78 s of real wall clock (`dt_ms`, per-sample
medians 19.6-35.8 s); an early beat still reads 174 datagrams, but every
stalled beat delivers exactly 124-128 datagrams - the 128-unit cooperative
budget of one task poll - or 253-256, two polls.  That caps inbound reads at
about 6/s while the kernel receive queue sits pinned near its ~1 MB limit
and overflows (`kernel_drops` up to 2694, 50-270 per beat), so answers reach
the process with ages of p90 33 s / max 80 s (`ADNL late answer` counts
376-518 per sample) and members that answered other queries all along never
appear inside the phase window.

The mechanism is the cadence of tokio's IO driver.  A current-thread runtime
pulls the driver only every `event_interval` (61) task polls or when the
local run queue empties, and while the queue stays busy - eight scanner
samples on four shared vCPUs, with each poll carrying heavy work (debug
logging with hex formatting, hashing, allocation) that CPU oversubscription
stretches further - the window grows with the wall time of those 61 polls.
Two local measurements pin the window at this transport: with eight samples
pinned to two CPUs the demultiplexer's own scheduling gap covers the beat
(`gap_max_ms` 24.8 s inside a 26.1 s beat, `iters` a few dozen over the
datagram count), and a synthetic chain of two CPU-heavy tasks that keep the
local queue busy (each wakes the other and burns 100 ms before parking) held
a 10 ms timer for 6102 ms - exactly 61 polls of a 100 ms task.  Any task
parked on socket readiness waits for that window, and the demultiplexer was
one: probe answers sat in the kernel queue while the same peers answered
other queries, which is why every per-peer theory failed to explain the
uniform delay.

`AdnlUdpTransport` now reads the shared socket on a dedicated OS thread that
feeds a bounded (1024) channel; the channel send wakes the demultiplexer
through an ordinary task waker, and the drain is polled unconstrained so a
receive does not charge cooperative budget - both the wakeup and the re-arm
of the old path needed a driver or park turn.  The thread never waits: a
full channel drops the datagram and counts an `overrun`, the loss mode UDP
already has, and it stops when the demultiplexer exits (idle) or the
transport drops.  The regression test
`tonutils_adnl::adnl::udp::tests::inbound_routing_survives_a_busy_runtime`
keeps the driver window open with the same CPU-burning chain and asserts the
transport's own read-to-dispatch wait (`dispatch_wait_max_ms`, also reported
cumulatively as `wait_cum_max_ms` in the heartbeat) against half the window -
the leg this crate owns - rather than the round trip, which additionally
includes the waiter's own runtime wake: 2951 ms of a 3050 ms window on the
old code, 14 µs with the fix while the runtime stays hostile.

Run 39114243866 (2026-10-18, `378bba6`, 600 s budget): red again, and this
run isolates the remaining arm.  The receive thread engaged fully on the
kernel side - `kernel_drops` fell from 137663 (`0990ae3`) to 0, `queue_bytes`
stayed under 832 B in every beat (previously pinned at 0.72-1.05 MB),
session-queue drops fell from 412 to 6, and `overrun` stayed 0 - yet all
eight samples timed out with no inbound broadcast (`pushers=0`,
`custom_messages=0`, `arrived=0`), and `unrouted=0` shows that no push
reached the socket at all.  Admission was healthy in the same samples:
`unique_pingers` 7-21, `pings_received`/`pongs_sent` matched 1:1 at 36-126,
`address_storing_peers` 133-184, `membership_answers` 1528-1921; STUN
resolved the same endpoint-independent mapping in six samples
(`172.208.127.194:9232-9237`, Cloudflare and Google agreeing), and
`dht address publish` stored with `served_by_dht=true` in samples 1, 3, and
4, while `membership_listed` stayed 0 everywhere.  The answers that did
leave this process carried ages of p50 3.8 s / p90 40 s.  Upstream marks a
pending peer alive only when its probe is answered inside the deadline
(`OverlayImpl::process_pending_peer`, five seconds, `overlay-peers.cpp`);
`get_random_peer(only_alive)` then skips this node, so peers kept re-pinging
and never pushed - the "pingers present, pushers absent" arm.

Local measurement (2026-10-18, eight samples pinned to four CPUs with the
instrumented build, 240 s per sample): five of eight samples pushed and
delivered - three within 3-12 ms, two at 108-113 s - and three samples saw
no push at all.  Neither the kernel nor the demultiplexer channel lost
anything (`overrun=0`, `kernel_drops=0`, session-queue `dropped=0`,
`queue_max=0`), so the loss mode was pure latency.  The demultiplexer's
read-to-dispatch wait was p50 1497 ms / max 14574 ms per heartbeat;
session-queue waits above 1 s covered 375-705 of the 1171-3923 datagrams
each sample read (15-50 %), 61-115 of them past 5 s; the sampled answer age
(socket read to send) was p50 2088 ms / p90 7076 ms / max 11606 ms over 33
answered queries, seven of them past the five-second deadline.  Heartbeats
reported `dt_ms` bursts of 16-17 s, and several sessions logged ~10.1 s
waits in the same poll: a global runtime stall, not a session or peer
property.  Every answered query in these logs was an `overlay.query`
wrapper (`0xccfd8443`) - the exact query upstream full nodes probe with.

The root cause is the executor, not the queue: the demultiplexer and every
shared-session receive loop ran on the same current-thread runtime as the
scanner's own work (peer growth, churn, broadcast bookkeeping), so each
inbound hop queued behind seconds of unrelated task polls.  The receive
thread removed the driver-gated *read*; the *runtime hop* after it remained.

The two-part fix: `crates/tonutils-adnl/src/adnl/udp/inbound.rs` runs a
process-wide current-thread runtime on a dedicated thread
(`adnl-udp-inbound`), and the transport demultiplexer and all shared-session
receive loops spawn there, so the first two hops leave the application
runtime entirely.  `AdnlUdpSession::register_query_answer(id, closure)` lets
a session answer matching queries synchronously in its receive task, before
the inbox; the mempool registers the probe constructors it can answer from
no shared state when an overlay session connects - `overlay.query`
(`0xccfd8443`), `overlay.queryWithExtra` (`0x94ffc3e9`), `overlay.ping`
(`0x690cb481`), and `dht.ping` (`0xcbeb3f18`) - bumping exactly the
counters the consumer path recorded.  Peer liveness therefore no longer
depends on the consumer ever reaching the session inbox.  The behaviour is
pinned by `adnl::udp::tests::fast_answer` (answer bytes, decline cases,
counters), `tonutils_mempool::overlay_inbound::tests` (byte-exact parity
with `build_overlay_answer`), and the integration test
`overlay_session_fast_answers_a_wrapped_ping_without_a_consumer`.
Local measurement with the fix (2026-10-19, pin5, the same eight samples on
four pinned CPUs, 240 s each): the inbound path sits at the floor.  The
demultiplexer's read-to-dispatch wait per heartbeat was p50 1 ms / p90
42 ms / max 972 ms over 266 beats (p50 1497 ms / max 14574 ms before), and
every heartbeat fired on time - 10000-10003 ms intervals, against the
16-17 s `dt_ms` bursts recorded before.  The answers this process produced
ran p50 0 ms / max 784 ms with zero over the five-second deadline (p50
2088 ms / max 11606 ms and seven over five seconds before), and all 14
answered queries took the registered fast path: the consumer's
`answering overlay.ping`/`answering dht.ping` lines are absent from all
eight logs while `pongs_sent`/`pings_received` recorded each probe, one
`overlay.getRandomPeers` still fell through to the consumer and was
answered there, and no fast-answer send failed.  Loss stayed at zero
everywhere (`overrun`, `kernel_drops`, session-queue `dropped`,
`unrouted`), so the funnel counters keep their meaning across both paths.
The session-queue waits that remain (1-8 s, 15-71 datagrams per sample)
are the consumer's own inbox backpressure - the receive loop blocks on a
full inbox while the application runtime lags - which shows up in the age
of answers to this node's own queries (`ADNL answer age` p50 77 ms / p90
185 ms / 4 of 3820 past 5 s) and did not reach the pongs this run.

That run itself drew no delivery: all eight samples timed out at 240 s
with `pushers=0` and `arrived=0`.  Two environment facts separate the draw
from the transport.  The local egress now maps a different port per
destination - Cloudflare's STUN saw `31.40.152.253:5110`, two other
servers `:54915` and `:62132`, and Google's binding timed out - so the
publish step refused the endpoint-dependent value by design ("the NAT maps
per destination"; "no address published", and no `stored on`/`served_by_dht`
line appears in any sample), and with 1-5 pings answered per sample
(`unique_pingers` 0-4, `address_storing_peers` 61-79, `membership_answers`
889-1010 in the shorter window) no holder sampled this node into its
`propagate_broadcast_to` fan-out.  Delivery from this host is therefore
gated by its own NAT mapping and the upstream sampling lottery, and the
transport shows nothing lost in either direction; the CI runner's own NAT
and the 600 s window remain the delivery judge.

Run 39457557758 (2026-10-20, `f0c7790`, 600 s budget): green, the first
gate after the inbound-executor fix.  Sample 2 delivered at
`time_to_message=104.1s` and sample 7 at `127.7s`, so the job ended after
6m36s with the other six samples stopped healthy (`task_panics=0`
everywhere; `dht address publish: stored ... served_by_dht=true` in
samples 1, 3, 4, 5, and 8, sample 6 storing 0 of 4 candidates).  The
previous gate on the same overlay (39114243866, `378bba6`) was red with
`pushers=0` in all eight windows while its answers carried p50 3.8 s /
p90 40 s ages; the delta between the two runs is `f0c7790`
(`fix(adnl): answer peer probes off the application runtime`), whose
pinned local re-run measured p50 0 ms pong answers, p50 1 ms
read-to-dispatch, and heartbeats on time in every beat.

Run 39458941631 (2026-10-20, `3ea3706`, 600 s budget): green, the
docs-pointer follow-up of the same fix lineage.  Sample 7 delivered at
`time_to_message=29.4s` - the fastest recorded arm - and stopped the
remaining seven samples healthy (`task_panics=0`; `dht address publish:
stored ... served_by_dht=true` in samples 1, 3, 7, and 8).  With
39457557758 (104.1s / 127.7s) this is the second consecutive green gate
on `f0c7790`, against the red `pushers=0` baseline of 39114243866.

Local capture and churn runs (2026-10-20, `3ea3706` plus the working tree,
pin5 harness on this host, not CI): three local mainnet runs armed the new
opt-in receive-thread capture (`ADNL_UDP_CAPTURE_FILE`,
`ADNL_UDP_CAPTURE_MAX`) and fed both the fixture work and the peer-quality
decision.  A 2x150 s run (cap 4096) delivered in sample 2 at
`time_to_message=142.6s` (`accepted: 1`, `overlay_packets: 1`) and timed
out in sample 1 (`registered_peers=48`); a 2x300 s run (cap 16384, never
reached: 8567 and 4828 records) delivered in sample 2 at `140.0s`
(`accepted: 1`, `rejected: 1`, `invalid_warnings: 1`,
`overlay_packets: 2`) and timed out in sample 1 (`registered_peers=51`);
a churn-forcing 2x240 s run with the managed budget cut to 24
(`TON_MEMPOOL_LIVE_OVERLAY_MAX_PEERS=24`, 3009 and 2979 records) delivered
nothing (`accepted: 0` everywhere).  Together the runs recorded 27,575
capture records - raw wire datagrams, canonical `packetContents`
re-serializations, and three verbatim broadcast customs (one live
`overlay.broadcastFec` (`0xbad7c36a`), two `overlay.broadcast`
(`0xb15a2b6b`)) - which became the `live_wire` fixture files pinned
offline by `crates/tonutils-tl/src/tl/live_wire.rs`.  The transport
counters held (`overrun=0`, `kernel_drops=0`, heartbeats at
`dt_ms=1000x`), so these runs add no regression signal of their own.

The peer-quality census (`peer quality: snapshot ...`, printed on both
gate paths) reported `tracked=1 first=1 on_time=0 late=0 misses=0
evictions=0 best_score=2.00 worst_score=2.00` in each of the two
delivering windows and `tracked=0 ... evictions=0` in the four
non-delivering ones: delivery observations are the only source of quality
evidence, so an undelivered window arms nothing.  The churn run produced
the first live number for the over-budget condition quality eviction
enforces: the budget-24 roster ended the window at `registered_peers=28`
(last growth log: `added 6 session(s), registered_peers=22`), i.e. six
sessions re-registered above the cap after the last growth round - a
lost peer's reconnect re-registers without the growth gate, which is the
natural path that can arm eviction.  With `tracked=0`, no peer reached
`min_observations`, so `peer quality: evicting` stayed absent and
`evictions=0` in every window; the default decision and the numbers live
in TODO.md (quality items closed 2026-10-20).

Sample delivery ratio across the three runs: 2 of 6 windows delivered
(142.6 s and 140.0 s), consistent with the sampling lottery recorded
above rather than with any transport change.
