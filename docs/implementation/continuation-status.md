# Continuation status and controller checkpoints

[中文](continuation-status.zh-CN.md)

Goal Plan and Runtime Console show **Continuation conditions** separately from
Host delivery and Fresh turn. Ready means the existing operator declaration and
exact live authorized carrier satisfy the current readiness checks. It does not
guarantee a new turn: the chat application can delay dispatch or require user
confirmation. Not ready does not prohibit an ordinary manual continuation.
`accepted` is Host delivery acknowledgement; only the exact Wake consume proves
that this continuation turn ran. No new readiness field, authority or scheduler
is introduced.

For a controller that needs all selected workers, use the existing Goal-scoped
`AgentWait(mode=all)`: correlate exact Tasks and register the Wait **before**
starting execution. Partial terminals retain their individual facts without
per-Task controller Wakes; the final match creates one Wait Wake. Execution
Wakes remain separate. Reconcile `delivery_unknown` without another blind send.

## ChatGPT acceptance

Using an explicitly configured test controller, present its continuation card
and end the setup turn. On a later turn present an exact Goal Plan. Check that
Ready does not claim zero-click execution. Trigger one authorized test update;
inspect Host delivery and Fresh turn separately. If the Host asks for Send,
confirm deliberately, then verify that the resumed turn consumes the exact Wake.
For an ALL checkpoint, complete one of two selected workers: no controller
follow-up should occur until both have completed. Do not use production work.

Local DOM/React tests verify the wording and distinct acceptance/consume fields;
Store tests verify ALL attention suppression. They do not establish live Host
scheduling or modal frequency.
