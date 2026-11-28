#!/bin/sh
# Installed as /usr/local/bin/muster and /usr/local/bin/gh (ahead of /usr/bin
# on PATH). SSH sessions (Tailscale SSH) don't inherit the pod's environment,
# and non-interactive ones (`ssh muster@host 'gh ...'`) don't read
# /etc/profile.d either, so load the tokens and paths the entrypoint saved,
# then run the real binary. git's credential helper also goes through here.
if [ -r /run/muster/env ]; then
  # shellcheck disable=SC1091 # written at runtime by the entrypoint
  . /run/muster/env
fi
case "${0##*/}" in
  muster) exec /usr/local/libexec/muster "$@" ;;
  gh) exec /usr/bin/gh "$@" ;;
  *) echo "env-wrapper: unknown command ${0##*/}" >&2; exit 127 ;;
esac
