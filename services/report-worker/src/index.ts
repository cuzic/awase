export interface Env {
  REPORT_BUCKET: R2Bucket;
  RATE_LIMIT_KV: KVNamespace;
}

export const SCHEMA_VERSION = 3;
export const MAX_BODY_BYTES = 512 * 1024;
export const DAILY_REPORT_LIMIT_PER_IP = 20;
export const RELEASE_CACHE_KEY = "latest-release:v1";

const RELEASE_REFRESHING_KEY = "latest-release:refreshing";
const RELEASE_SOFT_TTL_SECONDS = 3600;
const RELEASE_CACHE_EXPIRATION_TTL_SECONDS = 86400;
const RELEASE_REFRESHING_TTL_SECONDS = 60;
const GITHUB_LATEST_RELEASE_URL = "https://api.github.com/repos/cuzic/awase/releases/latest";

type ImeKind = "Gji" | "MsIme" | "Unknown";
type KeyboardModel = "Jis" | "Us";
type SymptomCategory =
  | "WrongCharacterOutput"
  | "CharacterDropped"
  | "StuckInRomaji"
  | "UnexpectedWidthOrKana"
  | "ImeToggledUnexpectedly"
  | "ThumbKeyMisbehavior"
  | "BrokenAfterAppSwitch"
  | "BrokenAfterIdle"
  | "NoResponse"
  | "Other";

export interface BugReportPayload {
  schema_version: 3;
  app_version: string;
  os_version: string;
  ime_kind: ImeKind;
  ime_product_name: string | null;
  keyboard_model: KeyboardModel;
  windows_keyboard_layout: string;
  competing_software: string[];
  symptom_category: SymptomCategory;
  description: string;
  attach_log: boolean;
  log_excerpt: string | null;
  /** 実際の `log::` 出力（awase.log）の末尾。BUG-34 横展開で追加（後方互換のため
   * 省略可能扱い＝クライアントが送らなくても null 扱いで受理する。旧クライアントの
   * 報告を拒否しないため、他の必須フィールドと違い optionalNullableString で読む）。 */
  app_log_excerpt: string | null;
  attach_state_snapshot: boolean;
  state_snapshot: Record<string, unknown> | null;
  attach_config: boolean;
  config_toml: string | null;
  attach_layout: boolean;
  layout_yab: string | null;
  /** ADR-120 決定0a-report。schema_version は上げていないため、フィールド自体が
   * 存在しない旧クライアントの報告も受理する（app_log_excerpt と同じ理由、
   * optionalBoolean/optionalNullableRecord で読む）。 */
  attach_retro_eval_stats: boolean;
  retro_eval_stats: Record<string, unknown> | null;
  reported_at: string;
}

interface StoredReport {
  report_id: string;
  received_at: string;
  payload: BugReportPayload;
}

interface LatestReleaseCacheEntry {
  schema_version: 1;
  latest_version: string;
  checked_at: string;
  fetched_at: string;
}

interface LatestReleaseResponse {
  schema_version: 1;
  latest_version: string;
  checked_at: string;
  stale: boolean;
}

export class HttpError extends Error {
  constructor(
    public readonly status: number,
    message: string
  ) {
    super(message);
  }
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    return handleRequest(request, env, ctx);
  }
};

export async function handleRequest(
  request: Request,
  env: Env,
  ctx: ExecutionContext
): Promise<Response> {
  const url = new URL(request.url);

  if (url.pathname === "/v1/reports") {
    return handleReportIntake(request, env);
  }
  if (url.pathname === "/v1/latest-release") {
    return handleLatestRelease(request, env, ctx);
  }

  return jsonResponse({ error: "not_found" }, 404);
}

async function handleReportIntake(request: Request, env: Env): Promise<Response> {
  if (request.method !== "POST") {
    return jsonResponse({ error: "method_not_allowed" }, 405, {
      Allow: "POST"
    });
  }

  try {
    assertBodySizeFromContentLength(request.headers.get("Content-Length"), MAX_BODY_BYTES);
    const bodyText = await readBodyWithLimit(request, MAX_BODY_BYTES);
    const payload = parseAndValidatePayload(bodyText);

    const clientIp = request.headers.get("CF-Connecting-IP");
    if (!clientIp) {
      return jsonResponse({ error: "missing_client_ip" }, 400);
    }

    const rate = await incrementDailyRateLimit(
      env.RATE_LIMIT_KV,
      clientIp,
      new Date(),
      DAILY_REPORT_LIMIT_PER_IP
    );
    if (!rate.allowed) {
      return jsonResponse({ error: "rate_limit_exceeded" }, 429);
    }

    const reportId = generateUlid();
    const now = new Date();
    const key = reportObjectKey(reportId, now);
    const stored: StoredReport = {
      report_id: reportId,
      received_at: now.toISOString(),
      payload
    };

    await env.REPORT_BUCKET.put(
      key,
      JSON.stringify(stored, null, 2),
      {
        httpMetadata: {
          contentType: "application/json; charset=utf-8"
        }
      }
    );

    return jsonResponse({ report_id: reportId }, 201);
  } catch (error) {
    if (error instanceof HttpError) {
      return jsonResponse({ error: error.message }, error.status);
    }

    console.error("report intake failed", error);
    return jsonResponse({ error: "internal_server_error" }, 500);
  }
}

async function handleLatestRelease(
  request: Request,
  env: Env,
  ctx: ExecutionContext
): Promise<Response> {
  if (request.method !== "GET" && request.method !== "HEAD") {
    return jsonResponse({ error: "method_not_allowed" }, 405, {
      Allow: "GET, HEAD"
    });
  }

  // RATE_LIMIT_KV is named for report rate limits, but also stores the release cache by prefix.
  const cached = parseLatestReleaseCacheEntry(await env.RATE_LIMIT_KV.get(RELEASE_CACHE_KEY));
  if (cached === null) {
    const refreshed = await fetchAndCacheLatestRelease(env);
    if (refreshed === null) {
      return jsonResponse({ error: "upstream_unavailable" }, 503);
    }
    return jsonResponse(latestReleaseResponse(refreshed, false), 200);
  }

  if (isLatestReleaseFresh(cached, new Date())) {
    return jsonResponse(latestReleaseResponse(cached, false), 200);
  }

  ctx.waitUntil(refreshStaleLatestRelease(env));
  return jsonResponse(latestReleaseResponse(cached, true), 200);
}

async function refreshStaleLatestRelease(env: Env): Promise<LatestReleaseCacheEntry | null> {
  if (await env.RATE_LIMIT_KV.get(RELEASE_REFRESHING_KEY) !== null) {
    return null;
  }

  await env.RATE_LIMIT_KV.put(RELEASE_REFRESHING_KEY, "1", {
    expirationTtl: RELEASE_REFRESHING_TTL_SECONDS
  });
  return fetchAndCacheLatestRelease(env);
}

async function fetchAndCacheLatestRelease(env: Env): Promise<LatestReleaseCacheEntry | null> {
  try {
    const response = await fetch(GITHUB_LATEST_RELEASE_URL, {
      method: "GET",
      headers: {
        "User-Agent": "awase-update-check-worker (+https://awase.cc)",
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28"
      }
    });
    if (!response.ok) {
      return null;
    }

    const body: unknown = await response.json();
    if (!isRecord(body) || typeof body.tag_name !== "string") {
      return null;
    }

    const now = new Date().toISOString();
    const entry: LatestReleaseCacheEntry = {
      schema_version: 1,
      latest_version: normalizeReleaseVersion(body.tag_name),
      checked_at: now,
      fetched_at: now
    };

    await env.RATE_LIMIT_KV.put(RELEASE_CACHE_KEY, JSON.stringify(entry), {
      expirationTtl: RELEASE_CACHE_EXPIRATION_TTL_SECONDS
    });
    return entry;
  } catch (error) {
    console.error("latest release refresh failed", error);
    return null;
  }
}

function parseLatestReleaseCacheEntry(value: string | null): LatestReleaseCacheEntry | null {
  if (value === null) {
    return null;
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    return null;
  }

  if (
    !isRecord(parsed) ||
    parsed.schema_version !== 1 ||
    typeof parsed.latest_version !== "string" ||
    typeof parsed.checked_at !== "string" ||
    typeof parsed.fetched_at !== "string"
  ) {
    return null;
  }

  return {
    schema_version: 1,
    latest_version: parsed.latest_version,
    checked_at: parsed.checked_at,
    fetched_at: parsed.fetched_at
  };
}

function isLatestReleaseFresh(entry: LatestReleaseCacheEntry, now: Date): boolean {
  const fetchedAt = Date.parse(entry.fetched_at);
  return Number.isFinite(fetchedAt) && now.getTime() - fetchedAt < RELEASE_SOFT_TTL_SECONDS * 1000;
}

function latestReleaseResponse(
  entry: LatestReleaseCacheEntry,
  stale: boolean
): LatestReleaseResponse {
  return {
    schema_version: 1,
    latest_version: entry.latest_version,
    checked_at: entry.checked_at,
    stale
  };
}

function normalizeReleaseVersion(tagName: string): string {
  return tagName.startsWith("v") ? tagName.slice(1) : tagName;
}

export function parseContentLength(value: string | null): number | null {
  if (value === null) {
    return null;
  }

  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    throw new HttpError(400, "invalid_content_length");
  }

  return parsed;
}

export function assertBodySizeFromContentLength(value: string | null, maxBytes: number): void {
  const contentLength = parseContentLength(value);
  if (contentLength !== null && contentLength > maxBytes) {
    throw new HttpError(413, "request_body_too_large");
  }
}

export async function readBodyWithLimit(request: Request, maxBytes: number): Promise<string> {
  const body = await request.arrayBuffer();
  if (body.byteLength > maxBytes) {
    throw new HttpError(413, "request_body_too_large");
  }

  return new TextDecoder().decode(body);
}

export function parseAndValidatePayload(bodyText: string): BugReportPayload {
  let value: unknown;
  try {
    value = JSON.parse(bodyText);
  } catch {
    throw new HttpError(400, "invalid_json");
  }

  return validatePayload(value);
}

export function validatePayload(value: unknown): BugReportPayload {
  if (!isRecord(value)) {
    throw new HttpError(400, "payload_must_be_object");
  }

  if (value.schema_version !== SCHEMA_VERSION) {
    throw new HttpError(400, "unsupported_schema_version");
  }

  const appVersion = requiredString(value, "app_version");
  const osVersion = requiredString(value, "os_version");
  const imeKind = requiredImeKind(value.ime_kind);
  const imeProductName = requiredNullableString(value, "ime_product_name");
  const keyboardModel = requiredKeyboardModel(value.keyboard_model);
  const windowsKeyboardLayout = requiredString(value, "windows_keyboard_layout");
  if (windowsKeyboardLayout.length === 0) {
    throw new HttpError(400, "windows_keyboard_layout_required");
  }
  const competingSoftware = requiredStringArray(value, "competing_software");
  const symptomCategory = requiredSymptomCategory(value.symptom_category);
  const description = requiredString(value, "description").trim();
  if (symptomCategory === "Other" && description.length === 0) {
    throw new HttpError(400, "description_required_for_other_category");
  }

  const attachLog = requiredBoolean(value, "attach_log");
  const logExcerpt = requiredNullableString(value, "log_excerpt");
  // BUG-34 横展開: フィールド自体が存在しない（この変更より前のクライアント）
  // 場合も null として受理する。log_excerpt 等の既存必須フィールドと違い、
  // このフィールドを送らない旧クライアントの報告を拒否してはならない。
  const appLogExcerpt = optionalNullableString(value, "app_log_excerpt");
  const attachStateSnapshot = requiredBoolean(value, "attach_state_snapshot");
  const stateSnapshot = requiredNullableRecord(value, "state_snapshot");
  const attachConfig = requiredBoolean(value, "attach_config");
  const configToml = requiredNullableString(value, "config_toml");
  const attachLayout = requiredBoolean(value, "attach_layout");
  const layoutYab = requiredNullableString(value, "layout_yab");
  // ADR-120 決定0a-report: app_log_excerpt と同じ理由で、フィールド自体が
  // 存在しない旧クライアントの報告も拒否しない（schema_version は不変）。
  const attachRetroEvalStats = optionalBoolean(value, "attach_retro_eval_stats");
  const retroEvalStats = optionalNullableRecord(value, "retro_eval_stats");
  const reportedAt = requiredString(value, "reported_at");
  if (Number.isNaN(Date.parse(reportedAt))) {
    throw new HttpError(400, "reported_at_must_be_rfc3339");
  }

  if (!attachLog && logExcerpt !== null) {
    throw new HttpError(400, "log_excerpt_requires_attach_log");
  }
  if (!attachLog && appLogExcerpt !== null) {
    throw new HttpError(400, "app_log_excerpt_requires_attach_log");
  }
  if (!attachStateSnapshot && stateSnapshot !== null) {
    throw new HttpError(400, "state_snapshot_requires_attach_state_snapshot");
  }
  if (!attachConfig && configToml !== null) {
    throw new HttpError(400, "config_toml_requires_attach_config");
  }
  if (!attachLayout && layoutYab !== null) {
    throw new HttpError(400, "layout_yab_requires_attach_layout");
  }
  if (!attachRetroEvalStats && retroEvalStats !== null) {
    throw new HttpError(400, "retro_eval_stats_requires_attach_retro_eval_stats");
  }

  return {
    schema_version: SCHEMA_VERSION,
    app_version: appVersion,
    os_version: osVersion,
    ime_kind: imeKind,
    ime_product_name: imeProductName,
    keyboard_model: keyboardModel,
    windows_keyboard_layout: windowsKeyboardLayout,
    competing_software: competingSoftware,
    symptom_category: symptomCategory,
    description,
    attach_log: attachLog,
    log_excerpt: logExcerpt,
    app_log_excerpt: appLogExcerpt,
    attach_state_snapshot: attachStateSnapshot,
    state_snapshot: stateSnapshot,
    attach_config: attachConfig,
    config_toml: configToml,
    attach_layout: attachLayout,
    layout_yab: layoutYab,
    attach_retro_eval_stats: attachRetroEvalStats,
    retro_eval_stats: retroEvalStats,
    reported_at: reportedAt
  };
}

export interface RateLimitResult {
  allowed: boolean;
  count: number;
  limit: number;
  key: string;
}

export interface RateLimitKv {
  get(key: string): Promise<string | null>;
  put(key: string, value: string, options: { expirationTtl: number }): Promise<void>;
}

export async function incrementDailyRateLimit(
  kv: RateLimitKv,
  clientIp: string,
  at: Date,
  limit: number
): Promise<RateLimitResult> {
  const key = await rateLimitKey(clientIp, at);
  const currentValue = await kv.get(key);
  const current = currentValue === null ? 0 : Number.parseInt(currentValue, 10);
  const next = Number.isFinite(current) && current >= 0 ? current + 1 : 1;

  if (next > limit) {
    return {
      allowed: false,
      count: next,
      limit,
      key
    };
  }

  await kv.put(key, String(next), {
    expirationTtl: secondsUntilNextUtcDay(at)
  });

  return {
    allowed: true,
    count: next,
    limit,
    key
  };
}

export async function rateLimitKey(clientIp: string, at: Date): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(clientIp)
  );
  return `report-rate:${utcDateStamp(at)}:${base64Url(digest)}`;
}

export function reportObjectKey(reportId: string, at: Date): string {
  const year = String(at.getUTCFullYear()).padStart(4, "0");
  const month = String(at.getUTCMonth() + 1).padStart(2, "0");
  return `reports/${year}/${month}/${reportId}.json`;
}

export function generateUlid(at: Date = new Date()): string {
  const alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  let time = at.getTime();
  const chars = new Array<string>(26);

  for (let i = 9; i >= 0; i -= 1) {
    chars[i] = alphabet[time % 32] ?? "0";
    time = Math.floor(time / 32);
  }

  const random = new Uint8Array(16);
  crypto.getRandomValues(random);
  for (let i = 10; i < 26; i += 1) {
    chars[i] = alphabet[(random[i - 10] ?? 0) & 31] ?? "0";
  }

  return chars.join("");
}

function requiredString(value: Record<string, unknown>, field: string): string {
  const fieldValue = value[field];
  if (typeof fieldValue !== "string") {
    throw new HttpError(400, `${field}_required`);
  }
  return fieldValue;
}

function requiredBoolean(value: Record<string, unknown>, field: string): boolean {
  const fieldValue = value[field];
  if (typeof fieldValue !== "boolean") {
    throw new HttpError(400, `${field}_required`);
  }
  return fieldValue;
}

function requiredNullableString(value: Record<string, unknown>, field: string): string | null {
  const fieldValue = value[field];
  if (fieldValue === null || typeof fieldValue === "string") {
    return fieldValue;
  }
  throw new HttpError(400, `${field}_required`);
}

/**
 * `requiredNullableString` と異なり、フィールド自体が存在しない（`undefined`）
 * 場合も `null` として受理する。新しいフィールドを追加するとき、既存の
 * （このフィールドをまだ送らない）クライアントの報告を拒否しないために使う
 * （BUG-34 横展開、app_log_excerpt で導入）。
 */
function optionalNullableString(value: Record<string, unknown>, field: string): string | null {
  const fieldValue = value[field];
  if (fieldValue === undefined || fieldValue === null) {
    return null;
  }
  if (typeof fieldValue === "string") {
    return fieldValue;
  }
  throw new HttpError(400, `${field}_invalid`);
}

function requiredNullableRecord(
  value: Record<string, unknown>,
  field: string
): Record<string, unknown> | null {
  const fieldValue = value[field];
  if (fieldValue === null || isRecord(fieldValue)) {
    return fieldValue;
  }
  throw new HttpError(400, `${field}_required`);
}

/**
 * `requiredBoolean` と異なり、フィールド自体が存在しない（`undefined`）場合は
 * `false` として受理する。ADR-120 決定0a-report で導入 —
 * `optionalNullableString` と同じ理由（旧クライアントの報告を拒否しない）。
 */
function optionalBoolean(value: Record<string, unknown>, field: string): boolean {
  const fieldValue = value[field];
  if (fieldValue === undefined) {
    return false;
  }
  if (typeof fieldValue === "boolean") {
    return fieldValue;
  }
  throw new HttpError(400, `${field}_invalid`);
}

/**
 * `requiredNullableRecord` と異なり、フィールド自体が存在しない（`undefined`）
 * 場合は `null` として受理する。ADR-120 決定0a-report で導入 —
 * `optionalNullableString` と同じ理由（旧クライアントの報告を拒否しない）。
 */
function optionalNullableRecord(
  value: Record<string, unknown>,
  field: string
): Record<string, unknown> | null {
  const fieldValue = value[field];
  if (fieldValue === undefined || fieldValue === null) {
    return null;
  }
  if (isRecord(fieldValue)) {
    return fieldValue;
  }
  throw new HttpError(400, `${field}_invalid`);
}

function requiredStringArray(value: Record<string, unknown>, field: string): string[] {
  const fieldValue = value[field];
  if (!Array.isArray(fieldValue) || fieldValue.some((item) => typeof item !== "string")) {
    throw new HttpError(400, `${field}_required`);
  }
  return fieldValue;
}

function requiredImeKind(value: unknown): ImeKind {
  if (value === "Gji" || value === "MsIme" || value === "Unknown") {
    return value;
  }
  throw new HttpError(400, "ime_kind_required");
}

function requiredKeyboardModel(value: unknown): KeyboardModel {
  if (value === "Jis" || value === "Us") {
    return value;
  }
  throw new HttpError(400, "keyboard_model_required");
}

function requiredSymptomCategory(value: unknown): SymptomCategory {
  if (
    value === "WrongCharacterOutput" ||
    value === "CharacterDropped" ||
    value === "StuckInRomaji" ||
    value === "UnexpectedWidthOrKana" ||
    value === "ImeToggledUnexpectedly" ||
    value === "ThumbKeyMisbehavior" ||
    value === "BrokenAfterAppSwitch" ||
    value === "BrokenAfterIdle" ||
    value === "NoResponse" ||
    value === "Other"
  ) {
    return value;
  }
  throw new HttpError(400, "invalid_symptom_category");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function jsonResponse(body: unknown, status: number, headers?: HeadersInit): Response {
  const responseHeaders = new Headers(headers);
  responseHeaders.set("Content-Type", "application/json; charset=utf-8");

  return new Response(JSON.stringify(body), {
    status,
    headers: responseHeaders
  });
}

function secondsUntilNextUtcDay(at: Date): number {
  const nextDay = Date.UTC(
    at.getUTCFullYear(),
    at.getUTCMonth(),
    at.getUTCDate() + 1,
    0,
    0,
    0,
    0
  );
  return Math.max(60, Math.ceil((nextDay - at.getTime()) / 1000));
}

function utcDateStamp(at: Date): string {
  const year = String(at.getUTCFullYear()).padStart(4, "0");
  const month = String(at.getUTCMonth() + 1).padStart(2, "0");
  const day = String(at.getUTCDate()).padStart(2, "0");
  return `${year}${month}${day}`;
}

function base64Url(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}
