#!/usr/bin/env bash
# Creates/updates the `registry-afixo` image-pull Secret that lets the cluster
# pull the private ghcr.io/afixo/services/* packages.
#
#   deploy/scripts/create-pull-secret.sh <PAT>              # production (namespace afixo)
#   deploy/scripts/create-pull-secret.sh <PAT> staging      # namespace afixo-staging
#
# <PAT> is a classic GitHub personal access token with ONLY the read:packages
# scope:  https://github.com/settings/tokens/new?description=afixo-k8s-image-pull&scopes=read:packages
#
# The token is never written to disk here; note it does land in your shell
# history — run with a leading space (fish: `set -U fish_history ''`, zsh/bash:
# HISTCONTROL=ignorespace) or clear it afterwards.
set -euo pipefail

PAT="${1:-}"
ENVIRONMENT="${2:-production}"
[[ -n "$PAT" ]] || { echo "usage: $0 <github PAT with read:packages> [production|staging]" >&2; exit 2; }
case "$ENVIRONMENT" in
  production) NS=afixo ;;
  staging)    NS=afixo-staging ;;
  *) echo "unknown environment '$ENVIRONMENT' (production|staging)" >&2; exit 2 ;;
esac

# ghcr ignores the username for PAT auth, but it must be non-empty.
USERNAME="${GITHUB_USER:-$(gh api user --jq .login 2>/dev/null || echo afixo)}"

kubectl get ns "$NS" >/dev/null 2>&1 || kubectl create ns "$NS"

# `create --dry-run | apply` so re-running with a new token rotates it in place.
kubectl -n "$NS" create secret docker-registry registry-afixo \
  --docker-server=ghcr.io \
  --docker-username="$USERNAME" \
  --docker-password="$PAT" \
  --dry-run=client -o yaml | kubectl apply -f -

echo "registry-afixo updated in namespace $NS (user $USERNAME)."
echo "Verifying the token can read the packages…"
BEARER=$(curl -fsS -u "$USERNAME:$PAT" \
  "https://ghcr.io/token?scope=repository:afixo/services/gateway:pull" 2>/dev/null | jq -r '.token // empty')
if [[ -n "$BEARER" ]] && curl -fsS -H "Authorization: Bearer $BEARER" \
     "https://ghcr.io/v2/afixo/services/gateway/tags/list" >/dev/null 2>&1; then
  echo "  ghcr.io accepts it for ghcr.io/afixo/services/gateway  ✓"
else
  echo "  ghcr.io refused or the package does not exist yet (first deploy.yml run creates it)."
  echo "  If the packages exist, check the token has read:packages and has not expired."
fi
