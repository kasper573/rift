#!/bin/bash
# Syncs the realm's roles and groups with the source of truth (roles.conf, generated from
# game/world) on every container start. A temporary admin service account is written straight to
# the database before the server boots and deleted once the sync ends, so no admin credential ever
# exists outside this container or outlives the sync. Everything else about the realm lives in the
# realm import, not here.
set -euo pipefail

bin=/opt/keycloak/bin
realm="${RIFT_AUTH_AUDIENCE:?}"

kcadm() { "$bin/kcadm.sh" "$@" --config /tmp/kcadm.config; }
log() { echo "[provision] $*"; }

# The sync runs as a re-invocation rather than a backgrounded function so `set -e` stays in
# force (bash disables it inside any `if`/`||` context, which would let a partial sync pass).
if [[ "${1:-}" == "--sync" ]]; then
  trap 'log "FAILED: the realm may be out of sync with roles.conf"' ERR

  for ((i = 0; i < 60; i++)); do
    if kcadm config credentials --server http://localhost:8080 --realm master \
      --client "$PROVISION_CLIENT" --secret "$PROVISION_SECRET" 2>/dev/null; then
      break
    fi
    sleep 5
  done

  while read -r kind name _ granted; do
    case "$kind" in
    role)
      if ! kcadm get "roles/$name" -r "$realm" >/dev/null 2>&1; then
        log "creating role $name"
        kcadm create roles -r "$realm" -s "name=$name"
      fi
      ;;
    group)
      if kcadm create groups -r "$realm" -s "name=$name" 2>/dev/null; then
        log "created group $name"
      fi
      for role in $granted; do
        kcadm add-roles -r "$realm" --gname "$name" --rolename "$role"
      done
      ;;
    esac
  done </opt/keycloak/roles.conf

  # Drop every provisioning account: leftovers from interrupted runs first, our own last.
  own=""
  while IFS=, read -r id name; do
    case "$name" in
    "$PROVISION_CLIENT") own="$id" ;;
    provision-*) kcadm delete "clients/$id" -r master ;;
    esac
  done < <(kcadm get clients -r master --fields id,clientId --format csv --noquotes)
  kcadm delete "clients/$own" -r master
  log "realm '$realm' synced from roles.conf"
  exit 0
fi

export PROVISION_CLIENT="provision-$SRANDOM"
export PROVISION_SECRET="$SRANDOM$SRANDOM$SRANDOM$SRANDOM"
"$bin/kc.sh" bootstrap-admin service --optimized \
  --client-id "$PROVISION_CLIENT" --client-secret:env PROVISION_SECRET

"$0" --sync &

"$bin/kc.sh" "$@" &
server=$!
trap 'kill -TERM "$server" 2>/dev/null' TERM INT
wait "$server" || true
wait "$server" || true
