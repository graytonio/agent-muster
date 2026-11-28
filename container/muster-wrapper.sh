#!/bin/sh
# Installed as /usr/local/bin/muster. SSH sessions (Tailscale SSH) don't
# inherit the pod's environment, so load the tokens and paths the entrypoint
# saved before running the real binary.
if [ -r /run/muster/env ]; then
  . /run/muster/env
fi
exec /usr/local/libexec/muster "$@"
