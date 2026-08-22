#!/usr/bin/env bash
# Post-deploy verification — each line isolates one hop. Run after any change
# to the edge or the gateway.
#
#   deploy/scripts/verify-edge.sh [production|staging]
#   CF_ACCESS_CLIENT_ID=… CF_ACCESS_CLIENT_SECRET=… deploy/scripts/verify-edge.sh
set -u

env="${1:-production}"
case "$env" in
  production) site="https://afixo.io"; api="https://api.afixo.io"
              origin="https://origin.afixo.io"; origin_api="https://origin-api.afixo.io" ;;
  staging)    site="https://staging.afixo.io"; api="https://api-staging.afixo.io"
              origin="https://origin-staging.afixo.io"; origin_api="https://origin-api-staging.afixo.io" ;;
  *) echo "unknown environment: $env" >&2; exit 1 ;;
esac

code() { curl -s -o /dev/null -w '%{http_code}' --max-time 10 "$@"; }

echo "1. site (Worker only)                     $(code "$site/")                                  want 200 (staging: 302 to Access login is fine)"
echo "2. console path end to end                $(code "$site/api/v1/health")                     want 200"
echo "3. machine host end to end                $(code "$api/v1/health")                          want 200"
echo "4. origin without Access token            $(code "$origin/v1/health")                       want 403 (Access enforcing)"
echo "5. origin-api without Access token        $(code "$origin_api/v1/health")                   want 403 (Access enforcing)"
if [ -n "${CF_ACCESS_CLIENT_ID:-}" ]; then
echo "6. origin with Access token               $(code -H "CF-Access-Client-Id: $CF_ACCESS_CLIENT_ID" -H "CF-Access-Client-Secret: $CF_ACCESS_CLIENT_SECRET" "$origin/v1/health")   want 200"
fi
echo "7. machine host rejects no token          $(code "$api/v1/disclose/alice?purpose=shipping")  want 401"
echo "8. subject routes absent on machine host  $(code "$api/v1/personas")                        want 404"
echo "9. foreign Origin cannot write            $(code -X POST -H 'origin: https://evil.test' -H 'content-type: application/json' -d '{}' "$site/api/v1/rules")   want 403"
echo "10. nothing public resolves to the tunnel: dig +short $site $api → Cloudflare Worker (AAAA 100::), never <id>.cfargotunnel.com"
