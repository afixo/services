#!/usr/bin/env bash
# Creates the identity GitHub Actions deploys with, for ONE environment, and
# uploads its kubeconfig as the GitHub *environment* secret KUBE_CONFIG.
#
#   deploy/scripts/make-ci-kubeconfig.sh staging       # namespace afixo-staging
#   deploy/scripts/make-ci-kubeconfig.sh production    # namespace afixo
#
# Run it as the cluster admin (your own doctl kubeconfig). It applies, by hand
# and on purpose outside kustomize, a ServiceAccount `github-deployer` that can
# create/update the workloads in that namespace and nothing else: it cannot
# read Secrets (a leaked CI token exposes no database URL or OAuth secret),
# cannot delete, cannot touch other namespaces. Re-running it rotates nothing;
# to rotate, delete the `github-deployer-token` Secret first.
#
# The kubeconfig is written OUTSIDE the repository so it can never be committed.
set -euo pipefail

ENVIRONMENT="${1:-}"
case "$ENVIRONMENT" in
  production) NS=afixo ;;
  staging)    NS=afixo-staging ;;
  *) echo "usage: $0 staging|production" >&2; exit 2 ;;
esac
SA=github-deployer
SA_SECRET=github-deployer-token
OUT="${OUT:-$HOME/.afixo-ci-kubeconfig-$ENVIRONMENT.yaml}"

kubectl get ns "$NS" >/dev/null 2>&1 || kubectl create ns "$NS"

kubectl apply -f - <<YAML
apiVersion: v1
kind: ServiceAccount
metadata:
  name: $SA
  namespace: $NS
---
# Since Kubernetes 1.24 a ServiceAccount has no permanent token by default;
# this Secret asks for one and the cluster fills in .data.token.
apiVersion: v1
kind: Secret
metadata:
  name: $SA_SECRET
  namespace: $NS
  annotations:
    kubernetes.io/service-account.name: $SA
type: kubernetes.io/service-account-token
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: $SA
  namespace: $NS
rules:
  # Everything deploy/k8s/base renders, minus Secrets (created by
  # create-secrets.sh and cloudflare-tunnel-setup.sh, never by CI).
  - apiGroups: ["apps"]
    resources: ["deployments", "statefulsets"]
    verbs: ["get", "list", "watch", "create", "update", "patch"]
  - apiGroups: [""]
    resources: ["services", "configmaps"]
    verbs: ["get", "list", "watch", "create", "update", "patch"]
  - apiGroups: ["networking.k8s.io"]
    resources: ["networkpolicies"]
    verbs: ["get", "list", "watch", "create", "update", "patch"]
  # Read-only, so a failed rollout can explain itself in the CI log.
  - apiGroups: [""]
    resources: ["pods", "pods/log", "events"]
    verbs: ["get", "list", "watch"]
  - apiGroups: ["apps"]
    resources: ["replicasets"]
    verbs: ["get", "list", "watch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: $SA
  namespace: $NS
subjects:
  - kind: ServiceAccount
    name: $SA
    namespace: $NS
roleRef:
  kind: Role
  name: $SA
  apiGroup: rbac.authorization.k8s.io
---
# \`kubectl apply -k\` also sends the Namespace object (cluster-scoped), so the
# deployer needs get/patch on namespaces — restricted to this one by name.
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: $SA-namespace-$NS
rules:
  - apiGroups: [""]
    resources: ["namespaces"]
    resourceNames: ["$NS"]
    verbs: ["get", "patch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: $SA-namespace-$NS
subjects:
  - kind: ServiceAccount
    name: $SA
    namespace: $NS
roleRef:
  kind: ClusterRole
  name: $SA-namespace-$NS
  apiGroup: rbac.authorization.k8s.io
YAML

# The cluster address and CA come from the kubeconfig in use right now.
SERVER=$(kubectl config view --raw --minify -o jsonpath='{.clusters[0].cluster.server}')
CA=$(kubectl config view --raw --minify -o jsonpath='{.clusters[0].cluster.certificate-authority-data}')

TOKEN=""
for _ in 1 2 3 4 5; do
  TOKEN=$(kubectl -n "$NS" get secret "$SA_SECRET" -o jsonpath='{.data.token}' | base64 -d)
  [[ -n "$TOKEN" ]] && break
  sleep 1
done
if [[ -z "$TOKEN" ]]; then
  echo "ERROR: $NS/$SA_SECRET has no token yet; re-run in a few seconds." >&2
  exit 1
fi

umask 077
cat > "$OUT" <<YAML
apiVersion: v1
kind: Config
clusters:
  - name: afixo
    cluster:
      server: ${SERVER}
      certificate-authority-data: ${CA}
users:
  - name: ${SA}
    user:
      token: ${TOKEN}
contexts:
  - name: ci
    context:
      cluster: afixo
      user: ${SA}
      namespace: ${NS}
current-context: ci
YAML
echo "Wrote $OUT"

echo "Verifying the restricted identity:"
KUBECONFIG="$OUT" kubectl -n "$NS" get deploy >/dev/null && echo "  can list deployments   ✓"
if KUBECONFIG="$OUT" kubectl -n "$NS" get secret >/dev/null 2>&1; then
  echo "  WARNING: it can read Secrets — that must not happen." >&2; exit 1
else
  echo "  cannot read Secrets    ✓"
fi
if KUBECONFIG="$OUT" kubectl get ns >/dev/null 2>&1; then
  echo "  WARNING: it can list all namespaces." >&2; exit 1
else
  echo "  confined to $NS        ✓"
fi

if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
  # The environment must exist before a secret can be attached to it.
  gh api -X PUT "repos/{owner}/{repo}/environments/$ENVIRONMENT" --silent
  gh secret set KUBE_CONFIG --env "$ENVIRONMENT" < "$OUT"
  echo "Uploaded as environment secret KUBE_CONFIG on '$ENVIRONMENT'."
else
  echo "gh is not logged in. Paste the contents of $OUT as secret KUBE_CONFIG on the"
  echo "'$ENVIRONMENT' environment: Settings → Environments → $ENVIRONMENT → Add secret."
fi
