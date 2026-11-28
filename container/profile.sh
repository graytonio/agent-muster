# Login shells over Tailscale SSH get the pod's tokens and paths, so `gh`,
# `herdr` and `claude` work by hand the same way they do for muster.
if [ -r /run/muster/env ]; then
  # shellcheck disable=SC1091 # written at runtime by the entrypoint
  . /run/muster/env
fi
