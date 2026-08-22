#!/usr/bin/env bash
# Create/update the `afixo-secrets` Secret from a gitignored env file.
#
#   deploy/scripts/create-secrets.sh deploy/k8s/overlays/prod/secrets/.env.prod
#
# Required keys: AUTH_DATABASE_URL IDENTITY_DATABASE_URL POLICY_DATABASE_URL
#                AUDIT_DATABASE_URL GITHUB_CLIENT_ID GITHUB_CLIENT_SECRET
set -euo pipefail

file="${1:?usage: $0 <env-file>}"
ns="${NAMESPACE:-afixo}"

required=(AUTH_DATABASE_URL IDENTITY_DATABASE_URL POLICY_DATABASE_URL AUDIT_DATABASE_URL GITHUB_CLIENT_ID GITHUB_CLIENT_SECRET)
for key in "${required[@]}"; do
  if ! grep -Eq "^${key}=." "$file"; then
    echo "error: $key missing or empty in $file" >&2
    exit 1
  fi
done

for key in AUTH_DATABASE_URL IDENTITY_DATABASE_URL POLICY_DATABASE_URL AUDIT_DATABASE_URL; do
  url=$(grep -E "^${key}=" "$file" | cut -d= -f2-)
  case "$url" in
    postgres://*|postgresql://*) ;;
    *) echo "error: $key must be a postgres:// URL" >&2; exit 1 ;;
  esac
  case "$url" in
    *sslmode=require*) ;;
    *) echo "warning: $key has no sslmode=require — DigitalOcean managed Postgres needs it" >&2 ;;
  esac
done

kubectl get ns "$ns" >/dev/null 2>&1 || kubectl create ns "$ns"
kubectl -n "$ns" create secret generic afixo-secrets \
  --from-env-file="$file" \
  --dry-run=client -o yaml | kubectl apply -f -

echo "afixo-secrets updated in namespace $ns:"
kubectl -n "$ns" get secret afixo-secrets -o jsonpath='{.data}' | jq -r 'keys[]' | sed 's/^/  /'
echo "restart deployments to pick up changes: kubectl -n $ns rollout restart deploy"
