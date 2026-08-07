SHELL = /usr/bin/env bash -o pipefail
.SHELLFLAGS = -ec

LOCALBIN ?= $(shell pwd)/bin
$(LOCALBIN):
	mkdir -p $(LOCALBIN)

.PHONY: help
help: ## Display this help.
	@awk 'BEGIN {FS = ":.*##"; printf "\nUsage:\n  make \033[36m<target>\033[0m\n\n"} /^[a-zA-Z_0-9-]+:.*?##/ { printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } ' $(MAKEFILE_LIST)


RATCHET ?= $(LOCALBIN)/ratchet
RATCHET_VERSION ?= 0.12.0
RATCHET_V_BINARY := $(LOCALBIN)/ratchet-$(RATCHET_VERSION)

RATCHET_OS := $(shell uname -s | tr A-Z a-z)
RATCHET_ARCH := $(shell uname -m | sed 's/x86_64/amd64/; s/aarch64/arm64/')
RATCHET_URL := https://github.com/sethvargo/ratchet/releases/download/v$(RATCHET_VERSION)/ratchet_$(RATCHET_VERSION)_$(RATCHET_OS)_$(RATCHET_ARCH).tar.gz

GH_WORKFLOW_FILES := $(wildcard .github/workflows/*.yml) $(wildcard .github/workflows/*.yaml)

.PHONY: ratchet
ratchet: $(RATCHET_V_BINARY) ## Download ratchet locally if necessary.
$(RATCHET_V_BINARY): | $(LOCALBIN)
	curl -sSfL $(RATCHET_URL) | tar -xzO ratchet > $(RATCHET_V_BINARY)
	chmod +x $(RATCHET_V_BINARY)
	ln -sf ratchet-$(RATCHET_VERSION) $(RATCHET)

.PHONY: ratchet-pin
ratchet-pin: ratchet ## Pin GitHub Actions to commit SHAs.
	$(RATCHET) pin $(GH_WORKFLOW_FILES)

.PHONY: ratchet-update-all
ratchet-update-all: ratchet ## Update all pinned GitHub Actions to latest SHAs.
	$(RATCHET) update $(GH_WORKFLOW_FILES)

.PHONY: verify-ratchet
verify-ratchet: ratchet ## Verify GitHub Actions are pinned to commit SHAs.
	$(RATCHET) lint $(GH_WORKFLOW_FILES)
