## Git Rules

- Agents must work only on the `dev` branch.
- Before committing, agents must run `git branch --show-current` and verify that the current branch is `dev`.
- If the current branch is not `dev`, the agent must stop and report the problem.
- Agents must not commit to `main`.
- Agents must not switch branches unless the user explicitly asks.
- Agents must stage only task-owned files with explicit pathspecs.
- Agents must not use broad `git add .` or `git add -A` for task commits.
- Agents must inspect staged diff before committing.
- Agents must not push. The user performs all pushes manually.
- If verification fails or was not run, the agent must not commit unless the user explicitly allows it.

