// Command gen_go produces reference wire bytes for the cross-SDK TL fixture
// cases using tonutils-go's TL engine and its production DHT, overlay, ADNL,
// and message types (quic frames are registered from the upstream schema
// strings; see localMessageQuery below for the rationale).
//
// Modes:
//
//	go run .            verify every fixture's raw_hex against tonutils-go
//	go run . --write    regenerate raw_hex in the fixture files
//
// The fixture files define canonical logical fields; this generator
// serializes them through tonutils-go and compares the bytes with the
// committed expectation, so any layout divergence fails the CI job.
package main

import (
	"crypto/ed25519"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/xssnick/tonutils-go/adnl"
	"github.com/xssnick/tonutils-go/adnl/address"
	"github.com/xssnick/tonutils-go/adnl/dht"
	"github.com/xssnick/tonutils-go/adnl/keys"
	"github.com/xssnick/tonutils-go/adnl/overlay"
	"github.com/xssnick/tonutils-go/tl"
)

type fixtureCase struct {
	Name        string          `json:"name"`
	Constructor string          `json:"constructor"`
	References  []string        `json:"references"`
	Note        string          `json:"note,omitempty"`
	Fields      json.RawMessage `json:"fields"`
	RawHex      string          `json:"raw_hex"`
}

type fixtureFile struct {
	SchemaRevision string        `json:"schema_revision"`
	Cases          []fixtureCase `json:"cases"`
}

// Plain schema-level stand-ins registered through tonutils-go's TL engine.
//
// adnl.message.query/answer: tonutils-go's own MessageQuery type auto-boxes
// its query payload (multiple concatenated TL objects); these fixtures
// compare the schema-level `query:bytes` field against identical inputs in
// the other SDKs.
//
// quic.*: tonutils-go's adnl/quic package pulls in the quic-go-ton transport
// fork, which does not compile under Go 1.27 (its go1.27 build-tagged
// handshake file sets tls.QUICConfig.ClientHelloInfoConn, a field missing
// from the forked tls package). The transport is irrelevant for TL wire
// layout, so these fixtures use tonutils-go's engine with the exact
// upstream ton_api.tl schema strings instead; tonutils-go's own quic types
// (adnl/quic/tl.go) are defined as the same single `data:bytes` field.
type localMessageQuery struct {
	ID   []byte `tl:"int256"`
	Data []byte `tl:"bytes"`
}

type localMessageAnswer struct {
	ID   []byte `tl:"int256"`
	Data []byte `tl:"bytes"`
}

type localQuicMessage struct {
	Data []byte `tl:"bytes"`
}

type localQuicQuery struct {
	Data []byte `tl:"bytes"`
}

type localQuicAnswer struct {
	Data []byte `tl:"bytes"`
}

func init() {
	tl.Register(localMessageQuery{}, "adnl.message.query query_id:int256 query:bytes = adnl.Message")
	tl.Register(localMessageAnswer{}, "adnl.message.answer query_id:int256 answer:bytes = adnl.Message")
	tl.Register(localQuicMessage{}, "quic.message data:bytes = quic.Request")
	tl.Register(localQuicQuery{}, "quic.query data:bytes = quic.Request")
	tl.Register(localQuicAnswer{}, "quic.answer data:bytes = quic.Response")
}

func main() {
	fixturesDir := flag.String("fixtures", "../../../fixtures/cross_sdk", "directory with cross-sdk fixture JSON files")
	write := flag.Bool("write", false, "write computed raw_hex back into the fixture files")
	flag.Parse()

	paths, err := filepath.Glob(filepath.Join(*fixturesDir, "*.json"))
	if err != nil {
		fatal("glob fixtures: %v", err)
	}
	if len(paths) == 0 {
		fatal("no fixture files found in %s", *fixturesDir)
	}

	total, failed := 0, 0
	for _, path := range paths {
		if filepath.Base(path) == "manifest.json" {
			continue
		}
		raw, err := os.ReadFile(path)
		if err != nil {
			fatal("read %s: %v", path, err)
		}
		var fx fixtureFile
		if err := json.Unmarshal(raw, &fx); err != nil {
			fatal("parse %s: %v", path, err)
		}

		changed := false
		for i := range fx.Cases {
			tc := &fx.Cases[i]
			// Cases tonutils-go does not cover (for example live captures
			// with no canonical fields) are skipped like in gen_py.py.
			if !slices.Contains(tc.References, "tonutils-go") {
				fmt.Printf("SKIP %s (not covered by tonutils-go case inputs)\n", tc.Name)
				continue
			}
			total++
			got, err := serializeCase(tc)
			if err != nil {
				fmt.Printf("FAIL %s: %v\n", tc.Name, err)
				failed++
				continue
			}
			if *write {
				if tc.RawHex != got {
					tc.RawHex = got
					changed = true
				}
				fmt.Printf("WROTE %s (%d bytes)\n", tc.Name, len(got)/2)
				continue
			}
			if tc.RawHex == "" {
				fmt.Printf("FAIL %s: raw_hex is empty; run with --write to generate it\n", tc.Name)
				failed++
				continue
			}
			if tc.RawHex != got {
				fmt.Printf("FAIL %s: tonutils-go serialization diverges: %s\n", tc.Name, firstDiff(tc.RawHex, got))
				failed++
				continue
			}
			fmt.Printf("OK %s (%d bytes)\n", tc.Name, len(got)/2)
		}

		if changed {
			out, err := json.MarshalIndent(&fx, "", "  ")
			if err != nil {
				fatal("encode %s: %v", path, err)
			}
			out = append(out, '\n')
			if err := os.WriteFile(path, out, 0o644); err != nil {
				fatal("write %s: %v", path, err)
			}
		}
	}

	fmt.Printf("tonutils-go: %d cases, %d failed\n", total, failed)
	if failed > 0 {
		os.Exit(1)
	}
}

func serializeCase(tc *fixtureCase) (string, error) {
	value, err := buildValue(tc.Constructor, tc.Fields)
	if err != nil {
		return "", err
	}
	out, err := tl.Serialize(value, true)
	if err != nil {
		return "", fmt.Errorf("tonutils-go serialize: %w", err)
	}
	return hex.EncodeToString(out), nil
}

func buildValue(constructor string, fields json.RawMessage) (any, error) {
	f, err := decodeFields(fields)
	if err != nil {
		return nil, err
	}
	switch constructor {
	case "dht.findNode":
		return dht.FindNode{Key: hexField(f, "key"), K: int32Field(f, "k")}, nil
	case "dht.findValue":
		return dht.FindValue{Key: hexField(f, "key"), K: int32Field(f, "k")}, nil
	case "dht.ping":
		return dht.Ping{ID: int64Field(f, "random_id")}, nil
	case "dht.getSignedAddressList":
		return dht.SignedAddressListQuery{}, nil
	case "dht.nodes":
		nodes, err := buildDhtNodes(arrField(f, "nodes"))
		if err != nil {
			return nil, err
		}
		return dht.NodesList{List: nodes}, nil
	case "dht.node":
		return buildDhtNode(f)
	case "dht.valueFound":
		value, err := buildDhtValue(objField(f, "value"))
		if err != nil {
			return nil, err
		}
		return dht.ValueFoundResult{Value: value}, nil
	case "dht.valueNotFound":
		nodes, err := buildDhtNodes(arrField(f, "nodes"))
		if err != nil {
			return nil, err
		}
		return dht.ValueNotFoundResult{Nodes: dht.NodesList{List: nodes}}, nil
	case "overlay.getRandomPeers":
		peers, err := buildOverlayNodes(arrField(f, "peers"))
		if err != nil {
			return nil, err
		}
		return overlay.GetRandomPeers{List: overlay.NodesList{List: peers}}, nil
	case "overlay.ping":
		return overlay.Ping{}, nil
	case "overlay.query":
		return overlay.Query{Overlay: hexField(f, "overlay")}, nil
	case "quic.query":
		return localQuicQuery{Data: hexField(f, "data")}, nil
	case "quic.answer":
		return localQuicAnswer{Data: hexField(f, "data")}, nil
	case "quic.message":
		return localQuicMessage{Data: hexField(f, "data")}, nil
	case "adnl.message.query":
		return localMessageQuery{ID: hexField(f, "query_id"), Data: hexField(f, "query")}, nil
	case "adnl.message.answer":
		return localMessageAnswer{ID: hexField(f, "query_id"), Data: hexField(f, "answer")}, nil
	case "adnl.message.nop":
		return adnl.MessageNop{}, nil
	default:
		return nil, fmt.Errorf("unsupported constructor %q", constructor)
	}
}

func buildDhtNodes(items []any) ([]*dht.Node, error) {
	nodes := make([]*dht.Node, 0, len(items))
	for _, item := range items {
		node, err := buildDhtNode(asObject(item))
		if err != nil {
			return nil, err
		}
		nodes = append(nodes, node)
	}
	return nodes, nil
}

func buildDhtNode(f map[string]any) (*dht.Node, error) {
	addrList, err := buildAddrList(objField(f, "addr_list"))
	if err != nil {
		return nil, err
	}
	return &dht.Node{
		ID:        keys.PublicKeyED25519{Key: ed25519Key(strField(f, "id"))},
		AddrList:  addrList,
		Version:   int32Field(f, "version"),
		Signature: hexField(f, "signature"),
	}, nil
}

func buildAddrList(f map[string]any) (*address.List, error) {
	list := &address.List{
		Version:    int32Field(f, "version"),
		ReinitDate: int32Field(f, "reinit_date"),
		Priority:   int32Field(f, "priority"),
		ExpireAt:   int32Field(f, "expire_at"),
	}
	for _, item := range arrField(f, "addrs") {
		addr := asObject(item)
		ip, port, kind := parseAddr(addr)
		switch kind {
		case "udp":
			list.Addresses = append(list.Addresses, address.UDP{IP: ip, Port: port})
		case "quic":
			list.Addresses = append(list.Addresses, address.QUIC{IP: ip, Port: port})
		default:
			return nil, fmt.Errorf("unsupported address kind %q", kind)
		}
	}
	return list, nil
}

func parseAddr(addr map[string]any) (net.IP, int32, string) {
	if raw, ok := addr["udp"]; ok {
		udp := asObject(raw)
		return parseIP(strField(udp, "ip")), int32Field(udp, "port"), "udp"
	}
	if raw, ok := addr["quic"]; ok {
		quicAddr := asObject(raw)
		return parseIP(strField(quicAddr, "ip")), int32Field(quicAddr, "port"), "quic"
	}
	return nil, 0, ""
}

func parseIP(dotted string) net.IP {
	ip := net.ParseIP(dotted)
	if ip == nil {
		return nil
	}
	return ip.To4()
}

func buildDhtValue(f map[string]any) (dht.Value, error) {
	keyDesc := objField(f, "key_description")
	key := objField(keyDesc, "key")
	rule, err := buildUpdateRule(strField(keyDesc, "update_rule"))
	if err != nil {
		return dht.Value{}, err
	}
	return dht.Value{
		KeyDescription: dht.KeyDescription{
			Key: dht.Key{
				ID:    hexField(key, "id"),
				Name:  hexField(key, "name"),
				Index: int32Field(key, "idx"),
			},
			ID:         keys.PublicKeyED25519{Key: ed25519Key(strField(keyDesc, "id"))},
			UpdateRule: rule,
			Signature:  hexField(keyDesc, "signature"),
		},
		Data:      hexField(f, "value"),
		TTL:       int32Field(f, "ttl"),
		Signature: hexField(f, "signature"),
	}, nil
}

func buildUpdateRule(name string) (any, error) {
	switch name {
	case "signature":
		return dht.UpdateRuleSignature{}, nil
	case "anybody":
		return dht.UpdateRuleAnybody{}, nil
	case "overlayNodes":
		return dht.UpdateRuleOverlayNodes{}, nil
	default:
		return nil, fmt.Errorf("unsupported update rule %q", name)
	}
}

func buildOverlayNodes(items []any) ([]overlay.Node, error) {
	nodes := make([]overlay.Node, 0, len(items))
	for _, item := range items {
		f := asObject(item)
		nodes = append(nodes, overlay.Node{
			ID:        keys.PublicKeyED25519{Key: ed25519Key(strField(f, "id"))},
			Overlay:   hexField(f, "overlay"),
			Version:   int32Field(f, "version"),
			Signature: hexField(f, "signature"),
		})
	}
	return nodes, nil
}

func decodeFields(raw json.RawMessage) (map[string]any, error) {
	var f map[string]any
	if err := json.Unmarshal(raw, &f); err != nil {
		return nil, fmt.Errorf("decode fields: %w", err)
	}
	return f, nil
}

func asObject(v any) map[string]any {
	f, _ := v.(map[string]any)
	return f
}

func objField(f map[string]any, key string) map[string]any {
	return asObject(f[key])
}

func arrField(f map[string]any, key string) []any {
	items, _ := f[key].([]any)
	return items
}

func strField(f map[string]any, key string) string {
	s, _ := f[key].(string)
	return s
}

func hexField(f map[string]any, key string) []byte {
	decoded, err := hex.DecodeString(strField(f, key))
	if err != nil {
		return nil
	}
	return decoded
}

func ed25519Key(dotted string) ed25519.PublicKey {
	key, err := hex.DecodeString(dotted)
	if err != nil || len(key) != ed25519.PublicKeySize {
		return nil
	}
	return ed25519.PublicKey(key)
}

func int32Field(f map[string]any, key string) int32 {
	return int32(int64Field(f, key))
}

func int64Field(f map[string]any, key string) int64 {
	switch v := f[key].(type) {
	case float64:
		return int64(v)
	case json.Number:
		n, _ := v.Int64()
		return n
	}
	return 0
}

func firstDiff(expectedHex, gotHex string) string {
	expected, err1 := hex.DecodeString(expectedHex)
	got, err2 := hex.DecodeString(gotHex)
	if err1 != nil || err2 != nil {
		return fmt.Sprintf("invalid hex (expected %s, got %s)", short(expectedHex), short(gotHex))
	}
	limit := min(len(expected), len(got))
	for i := range limit {
		if expected[i] != got[i] {
			return fmt.Sprintf(
				"first difference at byte %d: expected ...%s..., got ...%s...",
				i, short(expectedHex[2*max(0, i-4):]), short(gotHex[2*max(0, i-4):]))
		}
	}
	return fmt.Sprintf("length differs: expected %d bytes, got %d", len(expected), len(got))
}

func short(hexStr string) string {
	if len(hexStr) > 24 {
		return hexStr[:24] + "..."
	}
	return strings.TrimSpace(hexStr)
}

func fatal(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "gen_go: "+format+"\n", args...)
	os.Exit(1)
}
