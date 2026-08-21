# AI Agent Control Center

A responsive Tauri desktop application for creating, configuring, monitoring,
and routing work between AI agents.

## Install on Arch Linux / KDE Plasma

Install the Tauri system requirements, Rust, and the official Codex CLI first.
Sign Codex in with the ChatGPT account that includes your Codex access:

```bash
curl -fsSL https://chatgpt.com/codex/install.sh | sh
codex login
```

Then install the desktop app:

```bash
npm run desktop:install
```

The installer builds the release application and adds **AI Agent Control
Center** to KDE's application menu with the included custom icon.

## Connect Codex and run an autonomous agent

1. Open **Settings → Codex connection** and confirm the runtime is connected.
2. In **Workspace manager**, add one or more existing project folders.
3. Open **Agents**, select an agent, and choose its Codex model and capabilities.
4. In the agent's **Tasks** tab, create a task, choose its workspace and routing
   mode, then select **Run Codex agent**.

The desktop app launches `codex exec` using the login already stored by the
official CLI. It does not ask for, store, or use an OpenAI API key. ChatGPT plan
usage limits still apply. Agent results are stored locally with the task so
they remain visible after restarting the app.

When upgrading from the API-based preview, the installer removes its obsolete
stored API credential automatically.

Web search is available only when the agent has internet access and its
internet approval policy is set to **Allow**, or when a matching one-time
authorization has been approved. Agents with write/full file access run in
Codex's `workspace-write` sandbox; other agents run read-only. The app does not
enable unrestricted system control or bypass Codex sandboxing.

## Safety and one-time approvals

Version 0.4 adds an execution gate between a task and Codex:

- **Balanced** respects each agent's capability and approval policies while
  forcing a review for destructive workspace actions.
- **Strict** requests approval before every file, terminal, web, or clipboard
  action.
- **Locked** permits inspection but blocks changes and external actions.

When a task needs permission, it is blocked before Codex starts and a record is
added to **Approvals** with its risk level, requested scopes, workspace, and
expiry time. An approval is valid for one matching task run, is consumed when
that run starts, and cannot authorize another run. Denied and expired requests
remain in the local audit history until cleared.

Privileged commands, operating-system package management, power operations,
and system control remain blocked in every mode. Destructive file work inside
the selected workspace can run only after a matching one-time approval. The
Codex sandbox remains the hard filesystem boundary; the approval screen is an
additional application-level safety control.

Version 0.4.1 also hides legacy workflow controls while an authorization is
pending or approved-but-unused, leaving only the valid next safety action.

## Multi-agent routing and senior review

Version 0.5 turns the agent list into an actual orchestration layer:

- **Automatic routing** assigns a new or pending task to the strongest active
  specialist using category expertise, capabilities, availability, and current
  workload. The task records why it was routed and which agent received it.
- **Senior review** launches a different senior, team-leader, or supervisor
  agent after the specialist finishes. The reviewer runs in Codex's read-only
  sandbox with web search and elevated authorizations disabled.
- A reviewer must return **Approved** or **Changes requested**. Requested
  changes return the task to the specialist with the review feedback attached;
  approval finishes the task.
- Review can be disabled, started manually, or launched automatically after
  each specialist run from **Settings → Routing and review**.

Revision feedback is checked by the same backend safety rules as the original
task. A reviewer can recommend changes, but it cannot grant permissions or
bypass the one-time approval system.

Version 0.5.1 keeps agent status and action controls on one stable row at
desktop and split-screen widths, then wraps them together only on small mobile
layouts.

While a task runs, the app streams Codex progress and exposes a **Stop agent**
control. Per-run timeouts are configured in Settings. Completed task cards list
changed files, open existing workspace items through the desktop, and show the
working-tree diff when the selected workspace is a Git repository.

## Development

```bash
npm ci
npm run desktop
```

## Remove the local installation

```bash
bash ./uninstall-kde.sh
```
