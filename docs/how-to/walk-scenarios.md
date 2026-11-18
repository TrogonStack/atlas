# Walk scenarios with example data

Validation checks that a model is well formed. Walking it with example data
checks that it works: that a timeline such as "submit review 42 with score
4, then approve it" gets every value it needs and ends in the state the
board claims. Use it to test a model before you build it, to check a
proposed change before you apply it, and to keep a model honest in CI.

The case format and every finding code are in the
[walk_scenarios reference](../reference/walk-scenarios.md).

## Write a case

Put cases next to the model they exercise. Write one `Specification` per
timeline, using the values a real run would use:

```yaml
apiVersion: eventmodel.atlas.trogonstack.com/v1alpha1
kind: Specification
metadata:
  namespace: acme-reviews
  name: order-42-review
spec:
  title: Delivered order 42 becomes eligible, then its buyer reviews it
  given:
    readModels:
    - readModel: orders.delivered-unreviewed
      rows:
      - {order_id: o-42, buyer_id: b-7, seller_id: s-3, status: delivered}
  steps:
  - command: grant-review-eligibility
    expect:
      emit: [review-eligibility.granted]
  - command: submit-review
    answer: {rating: 4, comment: Arrived on time.}
    expect:
      emit: [review.submitted]
  then:
    readModels:
    - readModel: eligible-reviews
      rows: []
```

Leave values out of `answer` when the model should supply them. In the
second step above, `order_id` must come from the `eligible-reviews` read
model that the review form observes; only the rating and comment are the
reviewer's input. Supplying a value in `answer` that the model is supposed
to carry hides exactly the gap the walk exists to find.

State `expect.reject` for timelines the model must refuse, such as acting
on a stream after its terminal event.

## Walk manifests offline

```sh
trogon-atlas walk-scenarios \
  -f examples/manifests/acme-reviews examples/manifests/acme-fulfillment \
  --cases examples/specifications
```

`-f` takes every namespace the cases reach. Nothing connects to a server.
Each finding prints on one line with its case and step, followed by a
summary of passed cases and coverage. Add `--format json` for the full
report as one `trogon-atlas.result.v1` document.

## Walk the live store

Omit `-f` to walk the stored model. By default the command fetches the
namespaces the cases name; pass `--namespace` (repeatable) to choose them,
for example to include an upstream namespace. `--branch` walks a branch.

From an agent, call the `walk_scenarios` MCP tool with `cases_yaml`. Set
`manifests_yaml` to walk a proposed change without applying it.

## Gate CI

The command exits `1` when any error finding is reported, so a CI step
fails on a broken timeline:

```sh
trogon-atlas walk-scenarios -f models/ --cases specifications/
```

Warnings (for example a step no scenario documents) and coverage findings
do not fail the run. Read the coverage summary to find storyboards and
scenarios that no case exercises yet, and add cases for them.
