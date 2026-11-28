#!/usr/bin/env bash
# Entrypoint for the muster server image. Runs as root under tini:
#   1. prepares the data dirs on the persistent volume
#   2. saves the pod's tokens for SSH sessions (/run/muster/env)
#   3. sets up Claude Code, git and gh for the muster user
#   4. clones MUSTER_REPOS that aren't cloned yet, marks every repo trusted
#      for Claude Code and installs pinned Rust toolchains
#   5. starts tailscaled with Tailscale SSH (when TS_AUTHKEY is set)
#   6. runs the herdr server as the muster user until the pod stops
#
# Environment:
#   MUSTER_DATA         persistent volume mount (default /var/lib/muster)
#   MUSTER_REPOS        space-separated owner/name GitHub repos to clone
#   MUSTER_GIT_NAME     commit author name for agent and muster commits
#   MUSTER_GIT_EMAIL    commit author email
#   GH_TOKEN            GitHub token for gh and git push
#   CLAUDE_CODE_OAUTH_TOKEN  from `claude setup-token` (or ANTHROPIC_API_KEY)
#   TS_AUTHKEY          Tailscale auth key; unset skips Tailscale
#   TS_HOSTNAME         tailnet hostname (default muster)
#   TS_EXTRA_ARGS       extra `tailscale up` flags
#   MUSTER_ENV_EXTRA    extra variable names to pass to SSH sessions
set -euo pipefail

DATA=${MUSTER_DATA:-/var/lib/muster}
HOME_DIR=$DATA/home

log() { echo "[muster-entrypoint] $*" >&2; }

as_muster() {
  setpriv --reuid=muster --regid=muster --init-groups \
    env HOME="$HOME_DIR" USER=muster LOGNAME=muster SHELL=/bin/bash "$@"
}

# --- 1. Data dirs ----------------------------------------------------------
# rustup's default homes (~/.rustup, ~/.cargo): every context runs as muster
# with this HOME, so they agree even in SSH commands with no pod environment.
# Older images used $DATA/rustup and $DATA/cargo; they are left in place.
install -d -o muster -g muster "$DATA" "$HOME_DIR" \
  "$DATA/state" "$DATA/logs" "$DATA/repos" "$DATA/worktrees" \
  "$HOME_DIR/.rustup" "$HOME_DIR/.cargo"
install -d -m 700 "$DATA/tailscale"
cd "$HOME_DIR"

# --- 2. Environment for SSH sessions ---------------------------------------
# Tailscale SSH sessions start with a clean login environment. The muster
# wrapper and /etc/profile.d/muster.sh source this file.
install -d -m 755 /run/muster
env_vars=(GH_TOKEN CLAUDE_CODE_OAUTH_TOKEN ANTHROPIC_API_KEY MUSTER_CONFIG_DIR
          MUSTER_DATA DISABLE_AUTOUPDATER HERDR_BIN GH_BIN ENTIRE_BIN
          PATH)
# shellcheck disable=SC2206
env_vars+=(${MUSTER_ENV_EXTRA:-})
(
  umask 027
  : > /run/muster/env
  for v in "${env_vars[@]}"; do
    if [ -n "${!v:-}" ]; then
      printf 'export %s=%q\n' "$v" "${!v}" >> /run/muster/env
    fi
  done
)
chown root:muster /run/muster/env
chmod 640 /run/muster/env

# --- 3. Claude Code, git, gh -----------------------------------------------
# Skip Claude Code's first-run onboarding (it would otherwise ignore the
# token and show the login screen) and the --dangerously-skip-permissions
# confirmation. Merges into existing files: they also hold per-project state.
# shellcheck disable=SC2016 # $1/$2 expand in the inner sh, on purpose
merge_json() { # file jq-filter
  local file=$1 filter=$2
  as_muster sh -c '[ -s "$1" ] || echo "{}" > "$1"' _ "$file"
  if as_muster jq -e 'type == "object"' "$file" >/dev/null 2>&1; then
    as_muster sh -c 'jq "$2" "$1" > "$1.tmp" && mv "$1.tmp" "$1"' _ "$file" "$filter"
  else
    log "warning: $file is not a JSON object; leaving it alone"
  fi
}
as_muster install -d -m 700 "$HOME_DIR/.claude"
merge_json "$HOME_DIR/.claude.json" '.hasCompletedOnboarding = true'
merge_json "$HOME_DIR/.claude/settings.json" '.skipDangerousModePermissionPrompt = true'

as_muster git config --global user.name "${MUSTER_GIT_NAME:-muster}"
as_muster git config --global user.email "${MUSTER_GIT_EMAIL:-muster@localhost}"
as_muster git config --global init.defaultBranch main
# git authenticates to GitHub through gh, and so GH_TOKEN. The helper is the
# /usr/local/bin/gh wrapper, not /usr/bin/gh (which `gh auth setup-git` would
# write), so it loads /run/muster/env in SSH sessions too. Replacing the
# entries also clears ones an older image wrote.
for host in https://github.com https://gist.github.com; do
  as_muster git config --global --replace-all "credential.$host.helper" ''
  as_muster git config --global --add "credential.$host.helper" '!/usr/local/bin/gh auth git-credential'
done
if [ -z "${GH_TOKEN:-}" ]; then
  log "warning: GH_TOKEN is not set; pushes and PRs will fail"
fi

# --- 4. Repos --------------------------------------------------------------
for repo in ${MUSTER_REPOS:-}; do
  dest="$DATA/repos/${repo##*/}"
  if [ -d "$dest/.git" ]; then
    continue
  fi
  log "cloning $repo into $dest"
  as_muster env GH_TOKEN="${GH_TOKEN:-}" gh repo clone "$repo" "$dest" \
    || log "warning: could not clone $repo (muster doctor will report it)"
done

# Claude Code asks "do you trust this folder?" the first time it starts in a
# repo, which muster sees as agent_not_ready. Run worktrees count as their
# main repo, so trusting each clone (and the parent dirs) is enough. A repo
# cloned by hand is trusted on the next pod start.
trust_dir() {
  merge_json "$HOME_DIR/.claude.json" \
    ".projects[$(jq -n --arg p "$1" '$p')].hasTrustDialogAccepted = true"
}
trust_dir "$DATA/repos"
trust_dir "$DATA/worktrees"
for dir in "$DATA"/repos/*/; do
  [ -d "$dir/.git" ] || continue
  dir=${dir%/}
  trust_dir "$dir"
  # Install the Rust toolchain a repo pins, so agents can build and test it
  # without each racing to download it.
  if [ -f "$dir/rust-toolchain.toml" ] || [ -f "$dir/rust-toolchain" ]; then
    log "installing the Rust toolchain for ${dir##*/}"
    # shellcheck disable=SC2016 # $1 expands in the inner sh
    as_muster sh -c 'cd "$1" && rustup toolchain install' _ "$dir" \
      || log "warning: could not install the Rust toolchain for ${dir##*/}"
  fi
done

# --- 5. Tailscale ----------------------------------------------------------
TAILSCALE_SOCKET=/run/tailscale/tailscaled.sock
tailscaled_pid=
if [ -n "${TS_AUTHKEY:-}" ]; then
  install -d -m 755 /run/tailscale
  # Userspace networking: no NET_ADMIN or /dev/net/tun. Tailscale SSH is
  # served by tailscaled itself, so SSH sessions land in this container.
  tailscaled --tun=userspace-networking --statedir="$DATA/tailscale" \
    --socket="$TAILSCALE_SOCKET" &
  tailscaled_pid=$!
  for _ in $(seq 1 30); do
    [ -S "$TAILSCALE_SOCKET" ] && break
    sleep 1
  done
  # shellcheck disable=SC2086
  tailscale --socket="$TAILSCALE_SOCKET" up --authkey="$TS_AUTHKEY" \
    --hostname="${TS_HOSTNAME:-muster}" --ssh ${TS_EXTRA_ARGS:-} \
    || log "warning: tailscale up failed; SSH access is unavailable"
else
  log "TS_AUTHKEY is not set; not starting Tailscale"
fi

# --- 6. herdr server -------------------------------------------------------
# shellcheck disable=SC2329 # invoked by the trap
shutdown() {
  log "stopping"
  as_muster herdr server stop || kill -TERM "$herdr_pid" 2>/dev/null || true
  wait "$herdr_pid" 2>/dev/null || true
  if [ -n "$tailscaled_pid" ]; then
    kill -TERM "$tailscaled_pid" 2>/dev/null || true
    wait "$tailscaled_pid" 2>/dev/null || true
  fi
  exit 0
}
trap shutdown TERM INT

# Agents in herdr panes inherit the server's environment, including the tokens.
as_muster herdr server &
herdr_pid=$!

for _ in $(seq 1 30); do
  if /usr/local/bin/muster-healthcheck; then
    break
  fi
  sleep 1
done

as_muster herdr integration install claude \
  || log "warning: herdr integration install claude failed"

# Report readiness in the pod log; failures here don't stop the pod.
as_muster /usr/local/bin/muster doctor || log "muster doctor reported problems (see above)"

# Exit (and let Kubernetes restart the pod) if herdr or tailscaled dies.
set +e
if [ -n "$tailscaled_pid" ]; then
  wait -n "$herdr_pid" "$tailscaled_pid"
else
  wait "$herdr_pid"
fi
status=$?
log "a supervised process exited with status $status"
shutdown_status=$status
as_muster herdr server stop >/dev/null 2>&1
[ -n "$tailscaled_pid" ] && kill -TERM "$tailscaled_pid" 2>/dev/null
exit "$shutdown_status"
