#!/bin/sh
# Liveness check: the herdr server for the muster user answers on its socket.
# (Not `muster doctor`: a missing repo clone shouldn't restart the pod and
# kill every running agent.) `herdr status server` exits 0 either way, so
# check the `running` field.
setpriv --reuid=muster --regid=muster --init-groups \
  env HOME="${MUSTER_DATA:-/var/lib/muster}/home" \
  herdr status server --json | jq -e '.running == true' >/dev/null
