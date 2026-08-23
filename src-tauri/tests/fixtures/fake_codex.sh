#!/usr/bin/env bash
set -u

scenario="${FAKE_CODEX_SCENARIO:-success}"

if [[ "${1:-}" == "--version" ]]; then
  if [[ "$scenario" == "unsupported_version" ]]; then
    printf '%s\n' 'not-a-codex-version'
  else
    printf '%s\n' 'codex-cli 1.2.3-fixture'
  fi
  exit 0
fi

if [[ "${1:-}" == "--help" ]]; then
  printf '%s\n' \
    '--ask-for-approval <POLICY> never' \
    '--search' \
    '--sandbox <MODE>' \
    '--disable <FEATURE>'
  exit 0
fi

if [[ "${1:-}" == "features" && "${2:-}" == "list" ]]; then
  printf '%s\n' \
    'apps stable true' \
    'browser_use stable true' \
    'hooks stable true' \
    'multi_agent stable true' \
    'plugins stable true' \
    'remote_plugin stable true' \
    'shell_snapshot stable true' \
    'shell_tool stable true' \
    'unified_exec stable true' \
    'workspace_dependencies stable true'
  exit 0
fi

if [[ "${1:-}" == "login" && "${2:-}" == "status" ]]; then
  if [[ "$scenario" == "missing_auth" ]]; then
    printf '%s\n' 'not signed in' >&2
    exit 1
  fi
  printf '%s\n' 'signed in'
  exit 0
fi

if [[ "${1:-}" == "exec" && "${2:-}" == "--help" ]]; then
  if [[ "$scenario" == "unsupported_flags" ]]; then
    printf '%s\n' '--ephemeral --ignore-user-config --ignore-rules --strict-config --skip-git-repo-check'
  else
    printf '%s\n' '--ephemeral --ignore-user-config --ignore-rules --strict-config --json --skip-git-repo-check'
  fi
  exit 0
fi

is_exec=false
for argument in "$@"; do
  if [[ "$argument" == "exec" ]]; then
    is_exec=true
    break
  fi
done

if [[ "$is_exec" != true ]]; then
  printf '%s\n' 'unsupported fake invocation' >&2
  exit 64
fi

/usr/bin/cat >/dev/null

emit_success() {
  printf '%s\n' \
    '{"type":"thread.started","thread_id":"thread-fixture"}' \
    '{"type":"turn.started"}' \
    '{"type":"item.started","item":{"id":"command-1","type":"command_execution"}}' \
    '{"type":"item.completed","item":{"id":"command-1","type":"command_execution"}}' \
    '{"type":"item.completed","item":{"id":"message-1","type":"agent_message","text":"fixture complete"}}' \
    '{"type":"turn.completed","usage":{"input_tokens":11,"output_tokens":7}}'
}

spawn_marker_child() {
  local marker="${FAKE_CODEX_MARKER:-task-0007-unmarked-child}"
  /bin/bash -c 'exec -a "$0" /usr/bin/sleep 30' "$marker" &
}

spawn_detached_marker_child() {
  local marker="${FAKE_CODEX_MARKER:-task-0007-unmarked-detached-child}"
  /usr/bin/setsid /bin/bash -c 'exec -a "$0" /usr/bin/sleep 30' "$marker" &
}

case "$scenario" in
  success)
    emit_success
    ;;
  nonzero)
    printf '%s\n' 'bounded fixture failure' >&2
    exit 7
    ;;
  malformed)
    printf '%s\n' '{not-json'
    ;;
  missing_final)
    printf '%s\n' \
      '{"type":"thread.started","thread_id":"thread-fixture"}' \
      '{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}'
    ;;
  huge_output)
    printf '%070000d' 0
    ;;
  descendant)
    spawn_marker_child
    emit_success
    ;;
  detached_descendant)
    spawn_detached_marker_child
    emit_success
    ;;
  cancel)
    spawn_detached_marker_child
    printf '%s\n' \
      '{"type":"thread.started","thread_id":"thread-cancel"}' \
      '{"type":"turn.started"}'
    /usr/bin/sleep 30
    ;;
  timeout)
    spawn_detached_marker_child
    printf '%s\n' \
      '{"type":"thread.started","thread_id":"thread-timeout"}' \
      '{"type":"turn.started"}'
    /usr/bin/sleep 30
    ;;
  *)
    emit_success
    ;;
esac
