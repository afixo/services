#!/usr/bin/env bash
# Create/update the `afixo-secrets` Secret from a gitignored env file.
#
#   deploy/scripts/create-secrets.sh deploy/k8s/overlays/prod/secrets/.env.prod              # namespace afixo
#   NAMESPACE=afixo-staging deploy/scripts/create-secrets.sh deploy/k8s/overlays/staging/secrets/.env.staging
#
# Required keys: POSTGRES_PASSWORD GITHUB_CLIENT_ID GITHUB_CLIENT_SECRET
# Optional:      AUTH_DATABASE_URL IDENTITY_DATABASE_URL POLICY_DATABASE_URL AUDIT_DATABASE_URL
#                — derived from POSTGRES_PASSWORD for the in-cluster Postgres
#                (postgres://afixo:<pw>@postgres:5432/afixo_<svc>) unless set.
# Generate a password with: openssl rand -base64 32 | tr -d '/+=' | cut -c1-32
set -euo pipefail

file="${1:?usage: $0 <env-file>}"
ns="${NAMESPACE:-afixo}"

get() { grep -E "^$1=" "$file" | tail -n1 | cut -d= -f2- || true; }

for key in POSTGRES_PASSWORD GITHUB_CLIENT_ID GITHUB_CLIENT_SECRET; do
  [[ -n "$(get "$key")" ]] || { echo "error: $key missing or empty in $file" >&2; exit 1; }
done
pw=$(get POSTGRES_PASSWORD)
case "$pw" in *[!A-Za-z0-9_.-]*)
  echo "error: POSTGRES_PASSWORD must be URL-safe ([A-Za-z0-9_.-]); it is embedded in the DATABASE_URLs" >&2; exit 1 ;;
esac

args=(--from-env-file="$file")
for svc in auth identity policy audit; do
  key="$(echo "$svc" | tr a-z A-Z)_DATABASE_URL"
  url=$(get "$key")
  if [[ -z "$url" ]]; then
    args+=(--from-literal="$key=postgres://afixo:$pw@postgres:5432/afixo_$svc")
  else
    case "$url" in
      postgres://*|postgresql://*) ;;
      *) echo "error: $key must be a postgres:// URL" >&2; exit 1 ;;
    esac
  fi
done

kubectl get ns "$ns" >/dev/null 2>&1 || kubectl create ns "$ns"
kubectl -n "$ns" create secret generic afixo-secrets "${args[@]}" \
  --dry-run=client -o yaml | kubectl apply -f -

echo "afixo-secrets updated in namespace $ns:"
kubectl -n "$ns" get secret afixo-secrets -o jsonpath='{.data}' | jq -r 'keys[]' | sed 's/^/  /'
echo "restart to pick up changes: kubectl -n $ns rollout restart deploy"
echo "NOTE: POSTGRES_PASSWORD is baked into the volume on first boot; changing it later means ALTER USER in psql too."
