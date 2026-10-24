---
name: grill-me
description: Get interviewed about a plan or idea, one question at a time, until every decision is made.
disable-model-invocation: true
---

# Grill me

The user has a plan, an idea or a decision that is not fully thought
through. Your job is to interview them until it is.

1. Ask the user, in one message, for the plan in their own words if they have
   not given it yet. Skip this if they already did.
2. Call the Skill tool with "grilling" and follow it for the whole session:
   one question at a time, each with your recommended answer.
3. Do not write code, specs or documents during the session. Collect
   decisions only.
4. When grilling stops, print its summary (decisions, deferred questions,
   assumptions).
5. Suggest a next step that fits what was decided:
   - a code change in this repository: `/to-spec` (or `/grill-with-docs` if
     the vocabulary was shaky);
   - a decision someone else must make: write the open questions as a short
     message they can answer;
   - stopping for now: `/handoff`.
