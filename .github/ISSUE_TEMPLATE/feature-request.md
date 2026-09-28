---
name: Feature request
about: Suggest something riff should do
title: "[Feature]: "
labels: ["enhancement"]
---

**What problem are you trying to solve?** Describe the situation, not the solution. "I have X and I want Y" says more than "add a Z button", and it is the part that survives if the proposed form turns out to be the wrong one.

**What would riff do instead?** The behavior you are asking for, concretely enough that someone could decide whether it was implemented. If it touches the UI, a sentence about where it would live is enough.

**What have you already tried?** Existing riff features that come close, workarounds you are using, or other players whose approach you would want.

**What else did you consider?** Other ways to solve the same problem, and why they are worse. If the honest answer is "nothing, there is no alternative", say that too.

## Out of scope

riff is offline-only by design, and that is a decision rather than a limitation waiting to be lifted: [`docs/product/decisions/001-offline-first.md`](docs/product/decisions/001-offline-first.md) records why. **Anything that needs a server, an account, or a network call will be declined** — online metadata or artwork lookup, streaming, scrobbling, cloud sync, and phone-home of any kind are all out. If your idea depends on one of those, the design argument is the interesting part, so open the issue and make it rather than sending a pull request.
