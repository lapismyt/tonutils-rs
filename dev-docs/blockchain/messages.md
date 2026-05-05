# Messages And Transactions

Messages trigger account transactions and are the main object users send to contracts.

## Message Families

TON messages are represented by TLB constructors. The major families are:

- internal messages between contracts,
- external inbound messages from outside the blockchain,
- external outbound messages emitted outside the blockchain.

## Common Message Info

Common message info stores routing and fee metadata. Fields vary by message family but can include:

- source address,
- destination address,
- value,
- import fee,
- ihr fee,
- forward fee,
- creation logical time,
- creation unix time,
- bounce flags.

## State Init

A message can carry `StateInit` to deploy a contract. State init can include:

- split depth,
- special tick/tock flags,
- code cell,
- data cell,
- libraries.

## Body

The body is usually a cell. For wallet messages it often contains:

- operation code,
- query id,
- transfer parameters,
- comments or payload references.

## Transaction Relation

Each transaction has at most one inbound message and zero or more outbound messages. To confirm a sent external message, locate the transaction whose inbound message hash matches the sent message hash.

## SDK Requirements

- Build external inbound messages.
- Build internal messages.
- Compute message hash.
- Decode inbound and outbound messages from transactions.
- Track sent message inclusion.

## Missing Work

- TLB message codecs.
- Wallet-specific message builders.
- Transaction location helpers.
- Fee estimation helpers.
