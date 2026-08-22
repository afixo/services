#!/usr/bin/env bash
# One-time per environment: create the tunnel, route its two ORIGIN hostnames
# to it, and write the cluster secret cloudflared reads. Idempotent where
# cloudflared allows it.
#
#   deploy/scripts/cloudflare-tunnel-setup.sh production
#   deploy/scripts/cloudflare-tunnel-setup.sh staging
#
# Only origin hostnames are routed to a tunnel. The public hostnames
# (afixo.io, api.afixo.io, staging.afixo.io, api-staging.afixo.io) are Worker
# custom domains and must never point at the tunnel.
#
# Afterwards, in Cloudflare Zero Trust → Access (see afixo-api/docs/access.md):
#   1. Create a Service Token per environment (→ afixo-api Worker secrets
#      CF_ACCESS_CLIENT_ID / CF_ACCESS_CLIENT_SECRET).
#   2. Self-hosted application on EACH origin hostname below, ONE policy each:
#      action **Service Auth** (not Allow), include → that service token.
#   3. Never a wildcard *.afixo.io application.
set -euo pipefail

env="${1:?usage: $0 production|staging}"
case "$env" in
  production)
    name="afixo-fra1"; ns="afixo"
    hosts=("origin.afixo.io" "origin-api.afixo.io") ;;
  staging)
    name="afixo-fra1-staging"; ns="afixo-staging"
    hosts=("origin-staging.afixo.io" "origin-api-staging.afixo.io") ;;
  *) echo "unknown environment: $env" >&2; exit 1 ;;
esac

command -v cloudflared >/dev/null || { echo "cloudflared not installed (brew install cloudflared)" >&2; exit 1; }

# `cloudflared tunnel route dns` appends the zone of the origin certificate it
# is using — and `tunnel login` is a no-op when ~/.cloudflared/cert.pem already
# exists for ANOTHER zone, which silently creates origin.afixo.io.<other-zone>.
# So this script only ever uses a certificate dedicated to afixo.io.
export TUNNEL_ORIGIN_CERT="$HOME/.cloudflared/afixo.io.pem"
if [ ! -f "$TUNNEL_ORIGIN_CERT" ]; then
  cat >&2 <<EOF
No origin certificate for afixo.io at $TUNNEL_ORIGIN_CERT. Create one:
  mv ~/.cloudflared/cert.pem ~/.cloudflared/cert.pem.other 2>/dev/null   # keep any other zone's cert aside
  cloudflared tunnel login          # pick the afixo.io zone in the browser
  mv ~/.cloudflared/cert.pem $TUNNEL_ORIGIN_CERT
  mv ~/.cloudflared/cert.pem.other ~/.cloudflared/cert.pem 2>/dev/null
then re-run this script.
EOF
  exit 1
fi

if ! cloudflared tunnel list --name "$name" -o json | jq -e '.[0]' >/dev/null 2>&1; then
  cloudflared tunnel create "$name"
fi
tunnel_id=$(cloudflared tunnel list --name "$name" -o json | jq -r '.[0].id')
echo "tunnel $name = $tunnel_id"

for host in "${hosts[@]}"; do
  cloudflared tunnel route dns "$name" "$host" || true
done

creds="$HOME/.cloudflared/$tunnel_id.json"
[ -f "$creds" ] || { echo "credentials file $creds not found" >&2; exit 1; }

kubectl get ns "$ns" >/dev/null 2>&1 || kubectl create ns "$ns"
kubectl -n "$ns" create secret generic cloudflared-credentials \
  --from-file=credentials.json="$creds" \
  --from-literal=tunnel-id="$tunnel_id" \
  --dry-run=client -o yaml | kubectl apply -f -

echo
echo "done. next: kubectl -n $ns rollout restart deploy/cloudflared (if already deployed)"
for host in "${hosts[@]}"; do
  echo "verify:   curl -s -o /dev/null -w '%{http_code}\n' https://$host/v1/health   # 403 = Access enforcing"
done
