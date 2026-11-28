# Login shells over Tailscale SSH get the pod's tokens and paths, so `gh`,
# `herdr` and `claude` work by hand the same way they do for muster.
if [ -r /run/muster/env ]; then
  . /run/muster/env
fi
