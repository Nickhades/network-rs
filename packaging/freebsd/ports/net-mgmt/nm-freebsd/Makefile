PORTNAME=	nm-freebsd
DISTVERSION=	0.2.5
CATEGORIES=	net-mgmt

MAINTAINER=	zxy@example.com
COMMENT=	NetworkManager D-Bus compatibility layer for FreeBSD
WWW=		https://github.com/Nickhades/network-rs

LICENSE=	BSD2CLAUSE
LICENSE_FILE=	${WRKSRC}/LICENSE

BUILD_DEPENDS=	${LOCALBASE}/bin/dbus-daemon:devel/dbus
RUN_DEPENDS=	${LOCALBASE}/bin/dbus-daemon:devel/dbus

USES=		cargo dbus
USE_RC_SUBR=	nm_freebsd
USE_GITHUB=	yes
GH_ACCOUNT=	Nickhades
GH_PROJECT=	network-rs
GH_TAGNAME=	v${DISTVERSION}

SUB_FILES=	org.freedesktop.NetworkManager.service
SUB_LIST=	PREFIX=${PREFIX}

OPTIONS_DEFINE=		XFCE
# XFCE is off by default so the port can be built on servers and headless
# builders without GTK/XFCE development packages.
OPTIONS_DEFAULT=

XFCE_USES=		pkgconfig
XFCE_LIB_DEPENDS=	libgtk-3.so:graphics/gtk3 \
			libxfce4panel-2.0.so:sysutils/xfce4-panel

.include "cargo-crates.mk"
.include <bsd.port.options.mk>

CARGO_BUILD_TARGET_DIR=	${WRKDIR}/target

.if ${PORT_OPTIONS:MXFCE}
PLIST_SUB+=	XFCE=""
.else
PLIST_SUB+=	XFCE="@comment "
.endif

do-build:
	@cd ${WRKSRC} && ${SETENV} ${MAKE_ENV} ${CARGO_ENV} \
		${CARGO} build --release --locked --offline -p nm-freebsd-daemon \
		--target-dir ${CARGO_BUILD_TARGET_DIR}
	@cd ${WRKSRC} && ${SETENV} ${MAKE_ENV} ${CARGO_ENV} \
		${CARGO} build --release --locked --offline -p nm-dhcp-notify \
		--target-dir ${CARGO_BUILD_TARGET_DIR}
	@cd ${WRKSRC} && ${SETENV} ${MAKE_ENV} ${CARGO_ENV} \
		${CARGO} build --release --locked --offline -p nm-freebsd-cli \
		--target-dir ${CARGO_BUILD_TARGET_DIR}
.if ${PORT_OPTIONS:MXFCE}
	@cd ${WRKSRC} && ${SETENV} ${MAKE_ENV} ${CARGO_ENV} \
		${CARGO} build --release --locked --offline -p nm-xfce-plugin \
		--target-dir ${CARGO_BUILD_TARGET_DIR}
.endif

do-install:
	${INSTALL_PROGRAM} ${CARGO_BUILD_TARGET_DIR}/release/nm-freebsd-daemon \
		${STAGEDIR}${PREFIX}/bin/
	${INSTALL_PROGRAM} ${CARGO_BUILD_TARGET_DIR}/release/nm-freebsd-cli \
		${STAGEDIR}${PREFIX}/bin/
	${MKDIR} ${STAGEDIR}${PREFIX}/libexec
	${INSTALL_PROGRAM} ${CARGO_BUILD_TARGET_DIR}/release/nm-notify-dhcp \
		${STAGEDIR}${PREFIX}/libexec/nm-notify-dhcp
	${SED} 's|%%PREFIX%%|${PREFIX}|g' \
		${WRKSRC}/packaging/freebsd/libexec/nm-freebsd-dhcp-hook.in \
		> ${WRKDIR}/nm-freebsd-dhcp-hook
	${INSTALL_SCRIPT} ${WRKDIR}/nm-freebsd-dhcp-hook \
		${STAGEDIR}${PREFIX}/libexec/nm-freebsd-dhcp-hook
	${MKDIR} ${STAGEDIR}${PREFIX}/etc/dbus-1/system.d
	${INSTALL_DATA} ${FILESDIR}/org.freedesktop.NetworkManager.conf \
		${STAGEDIR}${PREFIX}/etc/dbus-1/system.d/
	${MKDIR} ${STAGEDIR}${PREFIX}/share/dbus-1/system-services
	${INSTALL_DATA} ${WRKDIR}/org.freedesktop.NetworkManager.service \
		${STAGEDIR}${PREFIX}/share/dbus-1/system-services/
.if ${PORT_OPTIONS:MXFCE}
	${MKDIR} ${STAGEDIR}${PREFIX}/lib/xfce4/panel/plugins
	${INSTALL_LIB} ${CARGO_BUILD_TARGET_DIR}/release/libnm_freebsd_xfce.so \
		${STAGEDIR}${PREFIX}/lib/xfce4/panel/plugins/
	${MKDIR} ${STAGEDIR}${PREFIX}/share/xfce4/panel/plugins
	${INSTALL_DATA} ${CARGO_BUILD_TARGET_DIR}/release/build/nm-xfce-plugin-*/out/nm_freebsd_xfce.desktop \
		${STAGEDIR}${PREFIX}/share/xfce4/panel/plugins/
.endif

.include <bsd.port.mk>
