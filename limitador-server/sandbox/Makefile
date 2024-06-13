SHELL := /bin/bash

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
PROJECT_PATH := $(patsubst %/,%,$(dir $(MKFILE_PATH)))

DOCKER ?= $(shell which docker 2> /dev/null || echo "docker")

all: help

##@ General

# The help target prints out all targets with their descriptions organized
# beneath their categories. The categories are represented by '##@' and the
# target descriptions by '##'. The awk commands is responsible for reading the
# entire set of makefiles included in this invocation, looking for lines of the
# file as xyz: ## something, and then pretty-format the target and help. Then,
# if there's a line with ##@ something, that gets pretty-printed as a category.
# More info on the usage of ANSI control characters for terminal formatting:
# https://en.wikipedia.org/wiki/ANSI_escape_code#SGR_parameters
# More info on the awk command:
# http://linuxcommand.org/lc3_adv_awk.php

help: ## Display this help.
	@awk 'BEGIN {FS = ":.*##"; printf "\nUsage:\n  make \033[36m<target>\033[0m\n"} /^[a-zA-Z_0-9-]+:.*?##/ { printf "  \033[36m%-30s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } ' $(MAKEFILE_LIST)

##@ Deployment Options

deploy-in-memory: clean ## Counters are held in Limitador (ephemeral)
	$(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-memory.yaml up

deploy-redis: clean ## Uses Redis to store counters
	$(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-redis.yaml up

deploy-redis-tls: clean ## Uses Redis with TLS and password protected to store counters
	$(MAKE) ca
	$(MAKE) redis-client-certs
	$(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-redis-tls.yaml up

deploy-redis-cached: clean ## Uses Redis to store counters, with an in-memory cache
	$(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-redis-cached.yaml up

deploy-redis-otel: clean ## Uses Redis to store counters, instrumented with opentelemetry
	$(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-redis-otel.yaml up

deploy-disk: clean ## Uses disk to store counters
	$(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-disk.yaml up

deploy-distributed: clean ## Counters are held in Limitador (ephemeral) but replicated to other Limitador servers.
	$(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-distributed.yaml up

deploy-distributed-3-node: clean ## Counters are held in Limitador (ephemeral) but replicated to 3 other local Limitador servers.
	$(DOCKER) compose -f docker-compose-envoy-3-node.yaml -f docker-compose-limitador-distributed-3-node.yaml up

##@ Helper targets

build: clean ## Build the "limitador-testing" image
	$(DOCKER) compose -f docker-compose-limitador-memory.yaml build

build-all-features: clean ## Build the image "limitador-testing-all-features" image
	$(DOCKER) compose -f docker-compose-limitador-distributed.yaml build

ca: ## Create CA cert
	openssl genrsa -out ca.key 2048
	openssl req -batch -new -x509 -nodes -key ca.key -sha256 -days 1024 -out ca.crt

redis-client-certs: ## Create CSR, then sign it with CA cert
	openssl req -subj '/CN=redis'  -newkey rsa:4096 -nodes \
			-sha256 \
			-days 3650 \
			-keyout redis.key \
			-out redis.csr
	chmod +r redis.key
	openssl x509 -req -in redis.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out redis.crt -days 500 -sha256

##@ Cleanning targets

redis-clean-certs: ## Clean certs
	- rm *.crt *.key *.pem *.csr

clean-containers: ## clean containers
	- $(DOCKER) compose down --volumes --remove-orphans
	- $(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-memory.yaml down --volumes --remove-orphans
	- $(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-redis.yaml down --volumes --remove-orphans
	- $(DOCKER)_compose -f docker-compose-envoy.yaml -f docker-compose-limitador-redis-cached.yaml down --volumes --remove-orphans
	- $(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-disk.yaml down --volumes --remove-orphans
	- $(DOCKER) compose -f docker-compose-envoy.yaml -f docker-compose-limitador-redis-otel.yaml down --volumes --remove-orphans
	- $(MAKE) cleancerts

clean: ## clean all
	- $(MAKE) clean-containers
	- $(MAKE) redis-clean-certs

GRPCURL=$(PROJECT_PATH)/bin/grpcurl
$(GRPCURL):
	$(call go-install-tool,$(GRPCURL),github.com/fullstorydev/grpcurl/cmd/grpcurl@v1.8.9)

.PHONY: grpcurl
grpcurl: $(GRPCURL) ## Download grpcurl locally if necessary.

.PHONY: ghz
ghz:
	$(call go-install-tool,$(PROJECT_PATH)/bin/ghz,github.com/bojand/ghz/cmd/ghz@latest) 

RPS?=1000
.PHONY: load-test
load-test: ghz
	# see https://ghz.sh/docs/load for usage details
	$(PROJECT_PATH)/bin/ghz 127.0.0.1:18081 --insecure \
	--call envoy.service.ratelimit.v3.RateLimitService.ShouldRateLimit \
	--async --concurrency=50 \
	--rps=$(RPS) \
	--total=$(RPS)0 \
	--data-file load-test.json

# go-install-tool will 'go install' any package $2 and install it to $1.
define go-install-tool
@[ -f $(1) ] || { \
set -e ;\
TMP_DIR=$$(mktemp -d) ;\
cd $$TMP_DIR ;\
go mod init tmp ;\
echo "Downloading $(2)" ;\
GOBIN=$(PROJECT_PATH)/bin go install $(2) ;\
rm -rf $$TMP_DIR ;\
}
endef
