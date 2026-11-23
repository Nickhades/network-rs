// NetworkManager FreeBSD GNOME Shell indicator.
//
// This extension is a raw D-Bus client. It does not use libnm: FreeBSD's
// GNOME packages are currently built without NetworkManager client support.
//
// All D-Bus calls are asynchronous. A synchronous call here would block the
// GNOME Shell main loop while the daemon waits for wpa_supplicant or DHCP.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Clutter from 'gi://Clutter';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import * as ModalDialog from 'resource:///org/gnome/shell/ui/modalDialog.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const NM = 'org.freedesktop.NetworkManager';
const NM_PATH = '/org/freedesktop/NetworkManager';
const DEVICE_IFACE = NM + '.Device';
const WIRELESS_IFACE = NM + '.Device.Wireless';
const AP_IFACE = NM + '.AccessPoint';
const SETTINGS_IFACE = NM + '.Settings';
const SETTINGS_CONNECTION_IFACE = NM + '.Settings.Connection';
const SETTINGS_PATH = NM_PATH + '/Settings';

const PROXY_FLAGS = Gio.DBusProxyFlags.NONE;
const CALL_FLAGS = Gio.DBusCallFlags.NONE;
const CALL_TIMEOUT = 15000;

function stringVariant(value) {
    return new GLib.Variant('s', value);
}

function bandLabel(frequency) {
    if (frequency >= 2400 && frequency <= 2500)
        return '2.4G';
    if (frequency >= 4900 && frequency <= 5924)
        return '5G';
    if (frequency >= 5925 && frequency <= 7125)
        return '6G';
    return '';
}

function signalIcon(strength) {
    if (strength >= 80)
        return 'network-wireless-signal-excellent-symbolic';
    if (strength >= 60)
        return 'network-wireless-signal-good-symbolic';
    if (strength >= 40)
        return 'network-wireless-signal-ok-symbolic';
    if (strength >= 20)
        return 'network-wireless-signal-weak-symbolic';
    return 'network-wireless-signal-none-symbolic';
}

function ssidToString(bytes) {
    try {
        return new TextDecoder().decode(Uint8Array.from(bytes));
    } catch {
        return bytes.map(byte => String.fromCharCode(byte)).join('');
    }
}

function uuid() {
    try {
        return GLib.uuid_string_random();
    } catch {
        return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, c => {
            const r = Math.floor(Math.random() * 16);
            return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
        });
    }
}

function unpack(value) {
    return value && typeof value.deep_unpack === 'function'
        ? value.deep_unpack()
        : value;
}

function callAsync(proxy, method, params = null, timeout = CALL_TIMEOUT) {
    return new Promise((resolve, reject) => {
        proxy.call(method, params, CALL_FLAGS, timeout, null, (source, res) => {
            try {
                resolve(source.call_finish(res));
            } catch (error) {
                reject(error);
            }
        });
    });
}

function newProxy(path, iface) {
    return new Promise((resolve, reject) => {
        Gio.DBusProxy.new_for_bus(
            Gio.BusType.SYSTEM, PROXY_FLAGS, null,
            NM, path, iface, null,
            (_source, res) => {
                try {
                    resolve(Gio.DBusProxy.new_for_bus_finish(res));
                } catch (error) {
                    reject(error);
                }
            });
    });
}

const NmIndicator = GObject.registerClass(
class NmIndicator extends PanelMenu.Button {
    constructor() {
        super(0.0, 'NetworkManager FreeBSD');

        this._manager = null;
        this._wireless = null;
        this._wifiPath = null;
        this._wifiDevice = null;
        this._proxies = new Map();
        this._profiles = [];
        this._refreshSource = 0;
        this._refreshGeneration = 0;
        this._dialog = null;

        this._box = new St.BoxLayout({
            style_class: 'nm-freebsd-indicator',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._icon = new St.Icon({
            icon_name: 'network-wireless-offline-symbolic',
            style_class: 'system-status-icon',
        });
        this._label = new St.Label({
            text: 'Wi-Fi',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._box.add_child(this._icon);
        this._box.add_child(this._label);
        this.add_child(this._box);

        this.menu.connect('open-state-changed', (_menu, open) => {
            if (open)
                this.refresh().catch(error => this._reportError(error));
        });

        this._connectService().catch(error => this._unavailable(error));
    }

    async _connectService() {
        this._manager = await newProxy(NM_PATH, NM);
        this._manager.connect('g-signal',
            (_proxy, _sender, signalName) => {
                if (signalName === 'StateChanged')
                    this._queueRefresh(100);
            });
        await this.refresh();
    }

    async _proxy(path, iface) {
        const key = iface + '|' + path;
        let promise = this._proxies.get(key);
        if (promise)
            return await promise;

        promise = newProxy(path, iface);
        this._proxies.set(key, promise);
        try {
            const proxy = await promise;
            if (iface === WIRELESS_IFACE) {
                proxy.connect('g-signal', (_p, _sender, signalName) => {
                    if (signalName === 'AccessPointAdded' ||
                        signalName === 'AccessPointRemoved' ||
                        signalName === 'PropertiesChanged')
                        this._queueRefresh(100);
                });
            } else if (iface === AP_IFACE) {
                proxy.connect('g-signal', (_p, _sender, signalName) => {
                    if (signalName === 'PropertiesChanged')
                        this._queueRefresh(150);
                });
            }
            return proxy;
        } catch (error) {
            this._proxies.delete(key);
            throw error;
        }
    }

    _property(proxy, name) {
        const value = proxy.get_cached_property(name);
        return value ? value.deep_unpack() : null;
    }

    async _apInfo(path) {
        try {
            const ap = await this._proxy(path, AP_IFACE);
            const ssidBytes = this._property(ap, 'Ssid');
            if (!ssidBytes || ssidBytes.length === 0)
                return null;
            const flags = Number(this._property(ap, 'Flags') ?? 0);
            const wpa = Number(this._property(ap, 'WpaFlags') ?? 0);
            const rsn = Number(this._property(ap, 'RsnFlags') ?? 0);
            const frequency = Number(this._property(ap, 'Frequency') ?? 0);
            return {
                path,
                ssid: ssidToString(ssidBytes),
                frequency,
                band: bandLabel(frequency),
                strength: Number(this._property(ap, 'Strength') ?? 0),
                secure: (flags & 1) !== 0 || wpa !== 0 || rsn !== 0,
            };
        } catch {
            return null;
        }
    }

    async _findWifi() {
        const reply = await callAsync(this._manager, 'GetDevices');
        const devices = reply.deep_unpack()[0] ?? [];
        for (const path of devices) {
            try {
                const device = await this._proxy(path, DEVICE_IFACE);
                if (this._property(device, 'DeviceType') !== 2)
                    continue;
                this._wifiPath = path;
                this._wifiDevice = device;
                this._wireless = await this._proxy(path, WIRELESS_IFACE);
                return true;
            } catch {
                // A device can disappear while enumerating.
            }
        }
        return false;
    }

    async refresh() {
        if (!this._manager)
            return;
        const generation = ++this._refreshGeneration;
        try {
            if (!await this._findWifi()) {
                if (generation === this._refreshGeneration)
                    this._renderNoWifi();
                return;
            }
            if (generation !== this._refreshGeneration)
                return;

            const reply = await callAsync(this._wireless, 'GetAllAccessPoints');
            const paths = reply.deep_unpack()[0] ?? [];
            const activePath = this._property(
                this._wireless, 'ActiveAccessPoint') ?? '/';
            const rawEntries = (await Promise.all(paths.map(path => this._apInfo(path))))
                .filter(info => info !== null);
            if (generation !== this._refreshGeneration)
                return;
            const entries = this._groupEntries(rawEntries, activePath);
            entries.sort((a, b) => {
                if (a.active && !b.active)
                    return -1;
                if (b.active && !a.active)
                    return 1;
                return b.strength - a.strength;
            });
            const profiles = await this._profilesInfo();
            if (generation !== this._refreshGeneration)
                return;
            this._render(entries, activePath, profiles);
        } catch (error) {
            if (generation === this._refreshGeneration)
                this._reportError(error);
        }
    }

    _reportError(error) {
        Main.notify('NetworkManager FreeBSD', String(error.message ?? error));
    }

    _clearMenu() {
        this.menu.removeAll();
    }

    _addHeader(text) {
        const item = new PopupMenu.PopupMenuItem(text, {reactive: false});
        item.label.add_style_class_name('popup-subtitle-menu-item');
        this.menu.addMenuItem(item);
    }

    _renderNoWifi() {
        this._label.set_text('Wi-Fi');
        this._icon.set_icon_name('network-wireless-offline-symbolic');
        this._clearMenu();
        this._addHeader('Wi-Fi');
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        this.menu.addMenuItem(
            new PopupMenu.PopupMenuItem('No Wi-Fi device', {reactive: false}));
    }

    _unavailable(error) {
        this._label.set_text('Wi-Fi');
        this._icon.set_icon_name('network-wireless-offline-symbolic');
        this._clearMenu();
        this._addHeader('NetworkManager FreeBSD');
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        this.menu.addMenuItem(
            new PopupMenu.PopupMenuItem('Service unavailable', {reactive: false}));
        if (error) {
            this.menu.addMenuItem(
                new PopupMenu.PopupMenuItem(String(error.message ?? error),
                    {reactive: false}));
        }
    }

    _groupEntries(entries, activePath) {
        const grouped = new Map();
        for (const info of entries) {
            const active = info.path === activePath;
            const existing = grouped.get(info.ssid);
            if (!existing) {
                grouped.set(info.ssid, {
                    ...info,
                    active,
                    bssids: 1,
                });
                continue;
            }
            existing.bssids++;
            if (active) {
                existing.path = info.path;
                existing.frequency = info.frequency;
                existing.band = info.band;
                existing.strength = info.strength;
                existing.secure = info.secure;
                existing.active = true;
            } else if (!existing.active && info.strength > existing.strength) {
                existing.path = info.path;
                existing.frequency = info.frequency;
                existing.band = info.band;
                existing.strength = info.strength;
                existing.secure = info.secure;
            }
        }
        return Array.from(grouped.values());
    }

    async _profilesInfo() {
        try {
            const settings = await this._proxy(SETTINGS_PATH, SETTINGS_IFACE);
            const reply = await callAsync(settings, 'ListConnections');
            const paths = reply.deep_unpack()[0] ?? [];
            const profiles = await Promise.all(paths.map(async path => {
                try {
                    const profile = await this._proxy(
                        path, SETTINGS_CONNECTION_IFACE);
                    const reply = await callAsync(profile, 'GetSettings');
                    const values = unpack(reply)[0] ?? {};
                    const connection = unpack(values.connection) ?? {};
                    const id = String(unpack(connection.id) ?? path);
                    return {path, id};
                } catch {
                    return null;
                }
            }));
            return profiles.filter(profile => profile !== null)
                .sort((a, b) => a.id.localeCompare(b.id));
        } catch {
            return [];
        }
    }

    async _deleteProfile(path) {
        try {
            const profile = await this._proxy(path, SETTINGS_CONNECTION_IFACE);
            await callAsync(profile, 'Delete');
            this._queueRefresh(250);
        } catch (error) {
            this._reportError(error);
        }
    }

    _render(entries, activePath, profiles = []) {
        const active = entries.find(info => info.active || info.path === activePath);
        this._label.set_text(active ? 'Wi-Fi: ' + active.ssid : 'Wi-Fi');
        this._icon.set_icon_name(active
            ? signalIcon(active.strength)
            : 'network-wireless-symbolic');

        this._clearMenu();
        this._addHeader(active ? 'Wi-Fi: ' + active.ssid : 'Wi-Fi');
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        if (entries.length === 0) {
            this.menu.addMenuItem(
                new PopupMenu.PopupMenuItem('No networks found', {reactive: false}));
        } else {
            for (const info of entries) {
                const prefix = info.active || info.path === activePath ? '\u2713 ' : '';
                const band = info.band ? '  ' + info.band : '';
                const suffix = band + (info.secure ? '  \u{1F512}' : '') +
                    (info.bssids > 1 ? '  (' + info.bssids + ')' : '');
                const item = new PopupMenu.PopupMenuItem(
                    prefix + info.ssid + suffix);
                item.connect('activate', () => this._selectAccessPoint(info));
                this.menu.addMenuItem(item);
            }
        }

        if (profiles.length > 0) {
            this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
            this._addHeader('Saved connections');
            for (const profile of profiles) {
                const item = new PopupMenu.PopupMenuItem(
                    'Forget: ' + profile.id);
                item.connect('activate', () => {
                    this._deleteProfile(profile.path)
                        .catch(error => this._reportError(error));
                });
                this.menu.addMenuItem(item);
            }
        }

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        const rescan = new PopupMenu.PopupImageMenuItem(
            'Rescan', 'view-refresh-symbolic');
        rescan.connect('activate', () => {
            this.scan().catch(error => this._reportError(error));
        });
        this.menu.addMenuItem(rescan);

        if (active) {
            const disconnect = new PopupMenu.PopupImageMenuItem(
                'Disconnect', 'network-offline-symbolic');
            disconnect.connect('activate', () => {
                this.disconnect().catch(error => this._reportError(error));
            });
            this.menu.addMenuItem(disconnect);
        }
    }

    _selectAccessPoint(info) {
        if (info.secure)
            this._promptPassword(info);
        else
            this.connectTo(info, null);
    }

    _promptPassword(info) {
        if (this._dialog)
            return;

        const dialog = new ModalDialog.ModalDialog({
            styleClass: 'nm-freebsd-password-dialog',
        });
        this._dialog = dialog;

        const label = new St.Label({
            text: 'Password for ' + info.ssid,
            style_class: 'nm-freebsd-password-label',
        });
        const entry = new St.PasswordEntry({
            hint_text: 'Password',
            can_focus: true,
            style_class: 'nm-freebsd-password-entry',
        });
        dialog.contentLayout.add_child(label);
        dialog.contentLayout.add_child(entry);

        let submitted = false;
        const submit = () => {
            if (submitted)
                return;
            const password = entry.get_text();
            if (password.length < 8) {
                Main.notify('NetworkManager FreeBSD',
                    'Wi-Fi password must be at least 8 characters.');
                entry.grab_key_focus();
                return;
            }
            submitted = true;
            dialog.close();
            this.connectTo(info, password);
        };

        dialog.setButtons([
            {label: 'Cancel', action: () => dialog.close()},
            {label: 'Connect', action: submit, default: true},
        ]);
        entry.clutter_text.connect('activate', submit);
        dialog.connect('closed', () => {
            this._dialog = null;
        });
        dialog.open();
        entry.grab_key_focus();
    }

    _settings(info, password) {
        const connection = {
            id: stringVariant(info.ssid),
            uuid: stringVariant(uuid()),
            type: stringVariant('802-11-wireless'),
        };
        const wireless = {
            ssid: new GLib.Variant('ay',
                Array.from(new TextEncoder().encode(info.ssid))),
            mode: stringVariant('infrastructure'),
        };
        const settings = {
            connection,
            '802-11-wireless': wireless,
        };
        if (password !== null) {
            settings['802-11-wireless-security'] = {
                'key-mgmt': stringVariant('wpa-psk'),
                psk: stringVariant(password),
            };
        }
        return settings;
    }

    async connectTo(info, password) {
        if (!this._manager || !this._wifiPath) {
            this._reportError(new Error('NetworkManager is not ready'));
            return;
        }
        try {
            const settings = this._settings(info, password);
            const args = new GLib.Variant('(a{sa{sv}}oo)',
                [settings, this._wifiPath, info.path]);
            await callAsync(this._manager,
                'AddAndActivateConnection', args, 20000);
            this._queueRefresh(500);
        } catch (error) {
            this._reportError(error);
            this._queueRefresh(500);
        }
    }

    async scan() {
        if (!this._wireless)
            return;
        await callAsync(this._wireless, 'RequestScan',
            new GLib.Variant('(a{sv})', [{}]));
        this._queueRefresh(1500);
    }

    async disconnect() {
        if (!this._manager || !this._wifiDevice)
            return;
        const active = this._property(this._wifiDevice, 'ActiveConnection');
        if (active && active !== '/') {
            await callAsync(this._manager, 'DeactivateConnection',
                new GLib.Variant('(o)', [active]));
            this._queueRefresh(500);
        }
    }

    _queueRefresh(delay = 200) {
        if (this._refreshSource) {
            GLib.source_remove(this._refreshSource);
            this._refreshSource = 0;
        }
        this._refreshSource = GLib.timeout_add(
            GLib.PRIORITY_DEFAULT, delay, () => {
                this._refreshSource = 0;
                this.refresh().catch(error => this._reportError(error));
                return GLib.SOURCE_REMOVE;
            });
    }

    _onDestroy() {
        if (this._refreshSource) {
            GLib.source_remove(this._refreshSource);
            this._refreshSource = 0;
        }
        if (this._dialog) {
            this._dialog.close();
            this._dialog = null;
        }
        this._proxies.clear();
        this._manager = null;
        this._wireless = null;
        this._wifiDevice = null;
        super._onDestroy();
    }
});

export default class NmFreeBsdExtension extends Extension {
    enable() {
        this._indicator = new NmIndicator();
        Main.panel.addToStatusArea(this.uuid, this._indicator);
    }

    disable() {
        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }
    }
}
