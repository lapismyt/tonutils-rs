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
4. Track inclusion by message hash.
5. Locate transaction in account history.

## Missing Work

- TLB message models.
- Wallet signing helpers.
- Fee estimation helpers.
- Message tracking API.
