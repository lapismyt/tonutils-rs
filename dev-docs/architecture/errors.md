# Error Model

TON SDK errors should preserve the subsystem boundary where the error occurred. This is required for retry policy, user diagnostics, and safe proof handling.

## Error Families

| Family | Examples | Retry behavior |
| --- | --- | --- |
| TL serialization | unknown constructor, invalid bytes padding, EOF | usually no |
| ADNL transport | IO error, integrity error, EOF, invalid handshake | retry another peer |
| LiteAPI server | `liteServer.error` | usually no |
| TVM data | malformed BoC, cell overflow, invalid address | no until input changes |
| Contract execution | non-zero get-method exit code | no, semantic result |
| Proof verification | invalid signature, invalid Merkle proof | no; mark peer/data untrusted |
| Balancer state | no alive peers, no archival peer | retry after reconnect |

## Design Rules

- Do not map all errors to `anyhow::Error` in public APIs.
- Preserve server error code and message.
- Preserve ADNL error variants for balancer retry decisions.
- Preserve TVM decode context when decoding cells, BoC, stack, or TLB data.
- Contract execution errors must include exit code and raw result bytes if available.

## Current Gaps

- `LiteError::ServerError` does not include a user-friendly display of code and message.
- Some TVM APIs return `anyhow::Error`.
- Balancer retry policy only partially distinguishes transport and semantic errors.
- Proof verification error types do not exist yet.
