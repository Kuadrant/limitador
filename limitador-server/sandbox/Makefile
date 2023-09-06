SHELL := /bin/bash

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
PROJECT_PATH := $(patsubst %/,%,$(dir $(MKFILE_PATH)))

DOCKER_COMPOSE ?= $(shell which docker-compose 2>/dev/null)
DOCKER ?= $(shell which docker 2>/dev/null || echo "docker")

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
	$(DOCKER_COMPOSE) -f docker-compose-envoy.yaml -f docker-compose-limitador-memory.yaml up

deploy-redis: clean ## Uses Redis to store counters
	$(DOCKER_COMPOSE) -f docker-compose-envoy.yaml -f docker-compose-limitador-redis.yaml up

deploy-redis-cached: clean ## Uses Redis to store counters, with an in-memory cache
	$(DOCKER_COMPOSE) -f docker-compose-envoy.yaml -f docker-compose-limitador-redis-cached.yaml up

deploy-disk: clean ## Uses disk to store counters
	$(DOCKER_COMPOSE) -f docker-compose-envoy.yaml -f docker-compose-limitador-disk.yaml up

deploy-infinispan: clean ## Uses Infinispan to store counters
	$(DOCKER_COMPOSE) -f docker-compose-envoy.yaml -f docker-compose-limitador-infinispan.yaml up

##@ Helper targets

build: ## Build "limitador-testing" image
	$(DOCKER) build -t limitador-testing -f ../../Dockerfile ../../

clean: ## clean all containers
	- $(DOCKER_COMPOSE) -f docker-compose-envoy.yaml -f docker-compose-limitador-memory.yaml down --volumes --remove-orphans
	- $(DOCKER_COMPOSE) -f docker-compose-envoy.yaml -f docker-compose-limitador-redis.yaml down --volumes --remove-orphans
	- $(DOCKER_COMPOSE) -f docker-compose-envoy.yaml -f docker-compose-limitador-redis-cached.yaml down --volumes --remove-orphans
	- $(DOCKER_COMPOSE) -f docker-compose-envoy.yaml -f docker-compose-limitador-infinispan.yaml down --volumes --remove-orphans
