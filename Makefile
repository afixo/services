# afixo-services — developer entrypoints. `make help` lists them.
SHELL := /bin/bash
.DEFAULT_GOAL := help

SERVICES := gateway auth identity policy disclosure audit
REGISTRY ?= registry.digitalocean.com/afixo
TAG      ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo dev)
# kubernetes targets: ENV=prod (namespace afixo) or ENV=staging (namespace afixo-staging)
ENV      ?= staging
NS       := $(if $(filter prod,$(ENV)),afixo,afixo-staging)

-include .env
export

## ---- code -------------------------------------------------------------------

check: ## cargo check everything
	cargo check --workspace --all-targets

fmt: ## format
	cargo fmt --all

lint: ## fmt check + clippy (what CI runs)
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

test: ## unit + property tests (no infra needed)
	cargo test --workspace

deny: ## licences / advisories / bans (needs cargo-deny)
	cargo deny check

## ---- local infra ------------------------------------------------------------

infra-up: ## postgres in docker
	docker compose up -d postgres

infra-down: ## stop and remove local infra (keeps volumes)
	docker compose down

infra-reset: ## stop and DELETE local data volumes
	docker compose down -v

migrate: ## apply every service's migrations to the local databases
	@for s in auth identity policy audit; do \
	  url_var=$$(echo $$s | tr a-z A-Z)_DATABASE_URL; \
	  echo "== $$s"; \
	  sqlx migrate run --source services/$$s/migrations --database-url "$${!url_var}"; \
	done

## ---- run --------------------------------------------------------------------

run-%: ## run one service, e.g. make run-policy (reads .env; maps <SVC>_DATABASE_URL → DATABASE_URL)
	@svc=$*; port=$$(case $$svc in auth) echo 50051;; identity) echo 50052;; policy) echo 50053;; disclosure) echo 50054;; audit) echo 50055;; *) echo "";; esac); \
	url_var=$$(echo $$svc | tr a-z A-Z)_DATABASE_URL; \
	DATABASE_URL="$${!url_var}" GRPC_ADDR="0.0.0.0:$$port" cargo run -p afixo-$$svc

dev: ## run all six services locally (Ctrl-C stops all)
	@trap 'kill 0' INT TERM; \
	for s in $(SERVICES); do $(MAKE) --no-print-directory run-$$s & done; wait

## ---- containers -------------------------------------------------------------

docker-%: ## build one image, e.g. make docker-gateway
	docker build --build-arg SERVICE=$* -t $(REGISTRY)/$*:$(TAG) .

docker-all: $(addprefix docker-,$(SERVICES)) ## build all images

## ---- kubernetes -------------------------------------------------------------

k8s-render: ## render the overlay for ENV (default staging) without touching the cluster
	kubectl kustomize deploy/k8s/overlays/$(ENV)

k8s-diff: ## what `make k8s-apply ENV=…` would change
	kubectl diff -k deploy/k8s/overlays/$(ENV) || true

k8s-apply: ## apply the overlay for ENV (CI: master → staging, dispatch → prod)
	kubectl apply -k deploy/k8s/overlays/$(ENV)

k8s-status: ## rollout status of every deployment in ENV
	@for s in cloudflared $(SERVICES); do kubectl -n $(NS) rollout status deploy/$$s --timeout=120s; done

help: ## this list
	@grep -E '^[a-zA-Z_%-]+:.*?## ' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

.PHONY: check fmt lint test deny infra-up infra-down infra-reset migrate dev docker-all k8s-render k8s-diff k8s-apply k8s-status help
