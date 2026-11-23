PREFIX?=	/usr/local
CARGO?=	cargo

.PHONY: all build test deps smoke desktop gnome-extension stage install uninstall clean help

all: build

build:
	$(CARGO) build --release --workspace

test:
	./scripts/build-test.sh

deps:
	./scripts/check-deps.sh --with-plugin

smoke:
	./scripts/smoke-test.sh --scan --plugin

signal:
	./scripts/signal-test.sh

desktop:
	./scripts/desktop-test.sh

gnome-extension:
	./scripts/install-gnome-extension.sh

stage:
	./scripts/install.sh --destdir build/stage --no-start --no-smoke

install:
	./scripts/install.sh --prefix $(PREFIX)

uninstall:
	./scripts/install.sh --uninstall --yes

clean:
	$(CARGO) clean

help:
	@echo "make build     build daemon and plugin"
	@echo "make test      fmt, clippy, tests, release build"
	@echo "make deps      check FreeBSD build dependencies"
	@echo "make smoke     read-only smoke test of a running service"
	@echo "make desktop   isolated Xvfb/XFCE screenshot test"
	@echo "make install   one-click install (run as root)"
	@echo "make stage     install into build/stage for inspection"
	@echo "make uninstall remove installed files"
