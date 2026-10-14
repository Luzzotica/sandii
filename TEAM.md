# Sandii team

Project **Sandii** · Goal: [`GOAL.md`](./GOAL.md)

| Agent | Role | Path |
| --- | --- | --- |
| **sandii-producer** | PoC — priorities only, **no game code** | `…/agents/sandii-producer` |
| **sandii** | Engineer — implement + tests | this repo |
| **sandii-tester** | Curious-user play | `…/agents/sandii-tester` |

## Communication

Producer dual-requests ship work:

- `k2 msg sandii --inbox` — implement  
- `k2 msg sandii-tester --inbox` — play matrix  

Eng owns code. Tester owns play reports. Producer does not implement.

Desktop: `cargo run --release --example sandbox_window`  
Web: `cd crates/sandii_web && env -u NO_COLOR trunk serve`
