# External Messages

External messages are used to send transactions to contracts, usually wallets.

## LiteAPI Function

```tl
liteServer.sendMessage body:bytes = liteServer.SendMsgStatus;
```

`body` is a serialized external message BoC.

## Required TLB Models

Message construction needs:

- `CommonMsgInfo`,
- external inbound message info,
- internal message info,
- `StateInit`,
- message body cell,
- wallet-specific signing payloads.

## Send Flow

1. Build message cell.
2. Serialize as BoC.
3. Send `liteServer.sendMessage`.
4. Return the opaque `liteServer.SendMsgStatus.status` submission status.

Wallet V5R1 and V4R2 helpers are accepted submission adapters for this flow:
they build and sign an external-in message, optionally include `StateInit` for
deploy or first-message workflows, submit exactly one BoC through
`ContractProvider::send_external_message_boc`, and surface the provider's
status or error. They do not prove transaction inclusion.

Post-submit confirmation remains a separate flow:

1. Track inclusion by message hash.
2. Locate transaction in account history.
3. Verify execution status and fees against the expected wallet/account state.

## Missing Work

- Fee estimation helpers.
- Message tracking API.
- Post-send transaction lookup and inclusion verification.
