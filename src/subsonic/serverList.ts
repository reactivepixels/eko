/**
 * Multi-server list — persists server metadata (name / baseUrl / username) in
 * localStorage and passwords in the macOS Keychain via the free `secret_*` commands.
 *
 * Key design decisions:
 *   - Each server gets a stable `id` (crypto.randomUUID or a Date-based fallback).
 *   - Non-secret fields live at `eko.servers` in localStorage (JSON array).
 *   - Passwords are stored under the Keychain key `navidrome-<id>`.
 *   - The active server id is stored at `eko.servers.active` in localStorage.
 *   - On first load, if the old single-server `eko.subsonic` key exists, it is
 *     migrated into the list automatically (backward-compat).
 *
 * This module is FREE (no `pro` feature required).
 */

import { invoke } from "@tauri-apps/api/core";

// ── Types ─────────────────────────────────────────────────────────────────────

export interface ServerEntry {
  id: string;
  name: string;
  baseUrl: string;
  username: string;
}

export interface ServerList {
  servers: ServerEntry[];
  activeId: string | null;
}

// ── Storage keys ──────────────────────────────────────────────────────────────

const SERVERS_KEY = "eko.servers";
const ACTIVE_KEY = "eko.servers.active";
/** The legacy single-server key — migrated on first load. */
const LEGACY_KEY = "eko.subsonic";

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    // Fallback for environments without crypto.randomUUID
    return `srv-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  }
}

function keychainKey(id: string): string {
  return `navidrome-${id}`;
}

// ── Persistence ───────────────────────────────────────────────────────────────

function loadServers(): ServerEntry[] {
  try {
    const raw = localStorage.getItem(SERVERS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (s): s is ServerEntry =>
        s !== null &&
        typeof s === "object" &&
        typeof (s as ServerEntry).id === "string" &&
        typeof (s as ServerEntry).name === "string" &&
        typeof (s as ServerEntry).baseUrl === "string" &&
        typeof (s as ServerEntry).username === "string",
    );
  } catch {
    return [];
  }
}

function saveServers(servers: ServerEntry[]): void {
  try {
    localStorage.setItem(SERVERS_KEY, JSON.stringify(servers));
  } catch {
    /* ignore quota errors */
  }
}

function loadActiveId(): string | null {
  try {
    return localStorage.getItem(ACTIVE_KEY);
  } catch {
    return null;
  }
}

function saveActiveId(id: string | null): void {
  try {
    if (id) {
      localStorage.setItem(ACTIVE_KEY, id);
    } else {
      localStorage.removeItem(ACTIVE_KEY);
    }
  } catch {
    /* ignore */
  }
}

// ── Migration ─────────────────────────────────────────────────────────────────

/**
 * Migrate the old single-server `eko.subsonic` entry into the server list.
 * Returns the migrated entry (if any) so the caller can retrieve its password.
 */
export async function migrateLegacyServer(): Promise<
  (ServerEntry & { password: string | null }) | null
> {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(LEGACY_KEY);
  } catch {
    return null;
  }
  if (!raw) return null;

  // There's already a server list — don't double-migrate.
  const existing = loadServers();
  if (existing.length > 0) {
    // Clean up the legacy key so we never try again.
    try {
      localStorage.removeItem(LEGACY_KEY);
    } catch {
      /* ignore */
    }
    return null;
  }

  try {
    const stored = JSON.parse(raw) as {
      baseUrl?: string;
      username?: string;
      password?: string;
    };
    if (!stored.baseUrl || !stored.username) return null;

    // Try to get password from Keychain (new shape) or inline (legacy shape).
    let password: string | null = stored.password ?? null;
    if (!password) {
      try {
        password = await invoke<string | null>("secret_get", { key: "navidrome" });
      } catch {
        password = null;
      }
    }

    // Build the new entry.
    const entry: ServerEntry = {
      id: makeId(),
      name: hostnameLabel(stored.baseUrl),
      baseUrl: stored.baseUrl,
      username: stored.username,
    };

    // Persist the new shape.
    saveServers([entry]);
    saveActiveId(entry.id);

    // Store password under the new keyed format.
    if (password) {
      try {
        await invoke("secret_set", { key: keychainKey(entry.id), value: password });
      } catch {
        /* ignore — caller will fall back to ConnectPanel */
      }
    }

    // Delete old Keychain entry + legacy localStorage key.
    try {
      await invoke("secret_delete", { key: "navidrome" });
    } catch {
      /* ignore */
    }
    try {
      localStorage.removeItem(LEGACY_KEY);
    } catch {
      /* ignore */
    }

    return { ...entry, password };
  } catch {
    return null;
  }
}

/** Make a human-readable label from a URL (e.g. "192.168.1.10:4533"). */
function hostnameLabel(url: string): string {
  try {
    const u = new URL(url);
    return u.host || url;
  } catch {
    return url;
  }
}

// ── CRUD ──────────────────────────────────────────────────────────────────────

/** Read the current server list from localStorage. */
export function getServerList(): ServerList {
  const servers = loadServers();
  const activeId = loadActiveId();
  // Guard: activeId must refer to an existing entry.
  const validActive =
    activeId && servers.some((s) => s.id === activeId) ? activeId : (servers[0]?.id ?? null);
  return { servers, activeId: validActive };
}

/** Add a new server entry and store its password in the Keychain. */
export async function addServer(
  opts: { name?: string; baseUrl: string; username: string },
  password: string,
): Promise<ServerEntry> {
  const servers = loadServers();
  const entry: ServerEntry = {
    id: makeId(),
    name: opts.name?.trim() || hostnameLabel(opts.baseUrl),
    baseUrl: opts.baseUrl,
    username: opts.username,
  };
  servers.push(entry);
  saveServers(servers);
  await invoke("secret_set", { key: keychainKey(entry.id), value: password });
  return entry;
}

/** Remove a server entry and delete its password from the Keychain. */
export async function removeServer(id: string): Promise<void> {
  const servers = loadServers().filter((s) => s.id !== id);
  saveServers(servers);
  // If we removed the active server, reset active to the first remaining.
  const activeId = loadActiveId();
  if (activeId === id) {
    saveActiveId(servers[0]?.id ?? null);
  }
  try {
    await invoke("secret_delete", { key: keychainKey(id) });
  } catch {
    /* ignore — Keychain may not have an entry for this id */
  }
}

/** Rename a server entry. */
export function renameServer(id: string, name: string): void {
  const servers = loadServers().map((s) => (s.id === id ? { ...s, name: name.trim() } : s));
  saveServers(servers);
}

/** Set the active server id. */
export function setActiveServerId(id: string): void {
  saveActiveId(id);
}

/** Retrieve the password for a server from the Keychain. */
export async function getServerPassword(id: string): Promise<string | null> {
  try {
    return await invoke<string | null>("secret_get", { key: keychainKey(id) });
  } catch {
    return null;
  }
}

// ── Pure logic (unit-testable, no Tauri invoke) ───────────────────────────────

/**
 * Pure function: apply an add to a server list.
 * Used by unit tests without Tauri.
 */
export function applyAddServer(servers: ServerEntry[], entry: ServerEntry): ServerEntry[] {
  return [...servers, entry];
}

/**
 * Pure function: apply a remove to a server list.
 */
export function applyRemoveServer(servers: ServerEntry[], id: string): ServerEntry[] {
  return servers.filter((s) => s.id !== id);
}

/**
 * Pure function: apply a rename to a server list.
 */
export function applyRenameServer(servers: ServerEntry[], id: string, name: string): ServerEntry[] {
  return servers.map((s) => (s.id === id ? { ...s, name } : s));
}

/**
 * Pure function: resolve the active id after a remove.
 * If the removed id was active, returns the first remaining id (or null).
 */
export function resolveActiveAfterRemove(
  servers: ServerEntry[],
  removedId: string,
  currentActiveId: string | null,
): string | null {
  if (currentActiveId !== removedId) return currentActiveId;
  return servers[0]?.id ?? null;
}

/**
 * Pure function: build a server list from a migrated legacy entry.
 * Used by unit tests.
 */
export function buildMigratedList(legacy: { baseUrl: string; username: string }): ServerEntry[] {
  return [
    {
      id: "migrated",
      name: hostnameLabel(legacy.baseUrl),
      baseUrl: legacy.baseUrl,
      username: legacy.username,
    },
  ];
}
