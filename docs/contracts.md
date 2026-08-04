# Contracts

`tonutils-contracts` provides low-level building blocks for TON contract
integrations: `ContractProvider`, raw get-method execution and result decoding,
TVM stack conversion traits, state-init address helpers, raw external-message
BoC routing, and `ContractBlueprint` for fixed code plus typed TL-B data.

Transport implementations for `LiteClient` and `LiteBalancer` are supplied by
`tonutils-liteclient`. There is no contract-description format or generated
wrapper API.
