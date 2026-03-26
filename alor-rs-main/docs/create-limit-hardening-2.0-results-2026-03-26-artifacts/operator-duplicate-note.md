# Operator Duplicate Note

Context:

- during the first fresh-path acceptance, two separate manual `create:limit` commands were sent from the same manual source:
  - `f4711fbe-6174-4cc0-8eb5-57ec907f0bf8`
  - `20f9cb69-ac40-41eb-8752-3a56ea7eec94`

Evidence:

- both requests appeared in `cmd.orders.7502MIW` as separate stream messages;
- each request received its own accepted ack;
- each request produced its own broker order id:
  - `2023555935792437596`
  - `2023555935792437604`

Interpretation:

- this was not a gateway-side duplication of one request;
- this was an operator-induced duplicate manual send during an ambiguous acceptance attempt.

Cleanup:

- the lingering first order `2023555935792437596` was later canceled successfully by:
  - `request_id = 2bda3071-0ccb-43d1-a24a-d4f82509d317`
- final state:
  - `status = canceled`
  - `filled = 0.0`

Operational takeaway:

- avoid acceptance flows that use `loop ... 1` and then tempt a manual second `place` while the first helper invocation is still unresolved;
- prefer explicit single-shot helper or fully manual step-by-step commands.
