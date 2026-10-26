# Historical ADR: Serialize accepted state through a project authority

Status: superseded by [ADR 0004](0004-messaging-product.md)

This is a historical decision summary, not an active requirement.

The former semantic-state product specified a Project Authority for accepted revisions and policy evaluation. That subsystem has been removed. Current mail publication uses short SQLite transactions and scoped dispatch locks under the messaging architecture.

The [original record at the last additive head](https://github.com/infagent/merl/blob/dc240a4bbb12a777679d591231a3c60aeb4bbe2d/docs/adr/0002-project-authority-concurrency.md) remains available in Git history. Follow [ADR 0004](0004-messaging-product.md) and the [current architecture](../architecture.md) for the implemented product.
