SHELL := /bin/bash
.DEFAULT_GOAL := help

CARGO ?= cargo
NPM ?= npm
BROWSER ?= edge
PROFILE ?= $(BROWSER)
EXTENSION_DIR ?= dist/extension
TABCTL := $(CARGO) run --manifest-path rust/Cargo.toml -p tabctl --

.PHONY: help dev-build dev-setup dev-up dev-profile-show dev-ping dev-list-all dev-run dev-run-release-like

help:
	@echo "tabctl local development targets"
	@echo "  make dev-build"
	@echo "  make dev-setup [BROWSER=edge PROFILE=edge EXTENSION_DIR=dist/extension]"
	@echo "  make dev-up [BROWSER=edge PROFILE=edge EXTENSION_DIR=dist/extension]"
	@echo "  make dev-profile-show [PROFILE=edge]"
	@echo "  make dev-ping [PROFILE=edge]"
	@echo "  make dev-list-all [PROFILE=edge]"
	@echo "  make dev-run CMD='list --all --json' [PROFILE=edge]"
	@echo "  make dev-run-release-like CMD='list --all --json' [PROFILE=edge]"
	@echo "  (override npm path when needed: NPM=~/.local/share/mise/shims/npm)"

dev-build:
	$(NPM) run build

dev-setup:
	$(TABCTL) setup --browser $(BROWSER) --name $(PROFILE) --extension-dir $(EXTENSION_DIR) --json

dev-up: dev-build dev-setup dev-ping

dev-profile-show:
	$(TABCTL) profile-show --json

dev-ping:
	$(TABCTL) --profile $(PROFILE) ping --json

dev-list-all:
	$(TABCTL) --profile $(PROFILE) list --all --json

dev-run:
	@test -n "$(CMD)" || (echo "Usage: make dev-run CMD='list --all --json' [PROFILE=edge]" && exit 1)
	$(TABCTL) --profile $(PROFILE) $(CMD)

dev-run-release-like:
	@test -n "$(CMD)" || (echo "Usage: make dev-run-release-like CMD='list --all --json' [PROFILE=edge]" && exit 1)
	TABCTL_AUTO_SYNC_MODE=release-like $(TABCTL) --profile $(PROFILE) $(CMD)
