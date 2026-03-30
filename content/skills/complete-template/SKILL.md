---
name: complete-template
description: Finish a Nanite init draft by reading the session artifact, asking only the missing questions, filling AI slots, and writing the final file without unresolved slot markers.
---

# Complete Template

Use this skill when Nanite prepared a draft file and a `.nanite/init/session.md` artifact for an interactive document-generation flow.

## Workflow

1. Read `.nanite/init/session.md`.
2. If the prompt or workspace mentions `.nanite/init/readme-validation.md`, read that too.
3. Inspect the listed `context_paths` plus the obvious project manifests.
4. Use the recorded answers in the session artifact first.
5. Ask additional follow-up questions only when the repo and recorded answers still leave a real gap.
6. Replace every `[[NANITE_SLOT:...]]` marker in the draft.
7. For `README.md`, follow the README brief, required sections, and validation rules exactly.
8. Write only the target file named in the session artifact.
9. Stop once the target file is complete.

## Rules

- Do not write the target file until every required slot is resolved.
- Do not ignore recorded answers from the session artifact.
- Do not ask a listed follow-up question again unless the repository and recorded answers still leave that slot unresolved.
- Remove optional slots by replacing their marker with an empty string if the user chooses to skip them.
- For `README.md`, do not invent extra top-level sections beyond the required section list in the session artifact.
- For `README.md`, use the badge line from the README brief verbatim when it is present; otherwise omit badges.
- For `README.md`, include a `- Trivia:` bullet only when the README brief includes a trivia link.
- If a README validation report is present, treat it as a mandatory normalization checklist for the rewrite.
- Do not modify unrelated files.
- Do not leave any `[[NANITE_SLOT:...]]` markers in the final output.
- Preserve the rest of the draft exactly unless the session artifact explicitly says otherwise.
