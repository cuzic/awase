export interface Env {
  REPORT_BUCKET: R2Bucket;
  RATE_LIMIT_KV: KVNamespace;
}

/** ADR-222: 4。`log_excerpt_gz` / `app_log_excerpt_gz`（gzip + base64）を追加し、
 * 非圧縮の `log_excerpt` / `app_log_excerpt` は 4 では常に null。 */
export const SCHEMA_VERSION = 4;
/** 受理する schema_version。3 は旧クライアント（v1 保守ラインを含め今後も送り続ける）。 */
export const SUPPORTED_SCHEMA_VERSIONS: readonly number[] = [3, 4];
/** ADR-222: 512KiB → 2MiB（10 分ぶんのログを gzip して送るため）。512KiB は ADR-095 実装時の
 * 暫定値で、Cloudflare 側の制約ではない。クライアント（awase-windows の `MAX_BODY_BYTES`）と同じ値。 */
export const MAX_BODY_BYTES = 2 * 1024 * 1024;
/** `log_excerpt_gz` / `app_log_excerpt_gz` 1 本あたりの base64 文字列の最大長。本体上限から、
 * 他の項目（状態・設定・配列ファイル等。実測で通常 数十 KB）の余裕 256KiB を引いた値。
 * 2 本の合計は本体上限（`readBodyWithLimit`）が別に抑える。 */
export const MAX_LOG_GZ_BASE64_CHARS = MAX_BODY_BYTES - 256 * 1024;
export const DAILY_REPORT_LIMIT_PER_IP = 20;
export const RELEASE_CACHE_KEY = "latest-release:v1";

const RELEASE_REFRESHING_KEY = "latest-release:refreshing";
const RELEASE_SOFT_TTL_SECONDS = 3600;
const RELEASE_CACHE_EXPIRATION_TTL_SECONDS = 86400;
const RELEASE_REFRESHING_TTL_SECONDS = 60;
const GITHUB_LATEST_RELEASE_URL = "https://api.github.com/repos/cuzic/awase/releases/latest";
// ライン別判定が必要なとき（クライアントが ?current_version= を送るとき）だけ使う。
// GitHub の「latest」は全体で1つしか無く、v1/v2両ラインを併走させると片方の
// リリースがもう片方の「latest」を覆い隠すため、全件リストから自ラインの最大値を選ぶ。
// per_page=100で、最終ページ（返却件数がper_page未満）に達するまで最大
// GITHUB_RELEASES_MAX_PAGES ページ分ページングする（opusコードレビュー指摘: 1ページ目
// 固定だと、v1/v2合計が100件を超えた場合に古い方のラインの最新リリースがページ外に
// 落ちて見えなくなる）。
const GITHUB_RELEASES_PER_PAGE = 100;
const GITHUB_RELEASES_MAX_PAGES = 5;
const GITHUB_RELEASES_LIST_URL = `https://api.github.com/repos/cuzic/awase/releases?per_page=${GITHUB_RELEASES_PER_PAGE}`;

/**
 * v1(保守)/v2(新アーキテクチャ)ラインの境界（2026-09-27 ユーザー決定）。
 *
 * v2はしばらく `1.90.0` 以降のマイナーバージョンとして走り、安定してから
 * `2.0.0` へ移行する予定。この定数は「境界より上か」だけを見るので、
 * 2.0.0移行後もそのまま動く（2.0.0 は 1.90.0 より大きいため自動的にv2ラインの
 * ままになる）——移行時にこの定数を変更する必要はない。
 */
const V2_LINE_MIN_VERSION: Semver = [1, 90, 0];

type Semver = readonly [number, number, number];
type ReleaseLine = "v1" | "v2";

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
  schema_version: 3 | 4;
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
  /** ADR-222（schema_version 4）。journal を gzip して base64 にしたもの。Worker は解凍せず、
   * 形式（文字種・長さ・gzip の先頭バイト）だけを検証して、そのまま保存する。 */
  log_excerpt_gz: string | null;
  /** 実際の `log::` 出力（awase.log）の末尾。BUG-34 横展開で追加（後方互換のため
   * 省略可能扱い＝クライアントが送らなくても null 扱いで受理する。旧クライアントの
   * 報告を拒否しないため、他の必須フィールドと違い optionalNullableString で読む）。 */
  app_log_excerpt: string | null;
  /** ADR-222（schema_version 4）。awase.log を gzip して base64 にしたもの。 */
  app_log_excerpt_gz: string | null;
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
  /** ADR-148。GJI/MS-IMEのキーマップ・キー割当て設定。`SCHEMA_VERSION`は
   * 上げていないため、旧クライアントが生成した報告にはこの3フィールドが
   * 存在しない（`retro_eval_stats`と同じ理由でoptionalとして読む）。 */
  attach_ime_keymap: boolean;
  gji_keymap: Record<string, unknown> | null;
  msime_key_assignment: Record<string, unknown> | null;
  /** ADR-148 Phase 2（2026-09-07追記）。旧UI（互換モード）の詳細キー
   * カスタマイズで無変換/変換キーに「IMEオン/オフ」が割当てられているかの
   * 検出結果。`attach_ime_keymap`に相乗り（新規フラグは追加しない）。 */
  legacy_msime_keymap: Record<string, unknown> | null;
  /** ADR196-T2 決定1e後半（2026-09-07追記）。学習表の採否・自己検証・同梱表との
   * 突き合わせ・指紋。`attach_ime_keymap`に相乗り（新規フラグは追加しない、
   * legacy_msime_keymapと同じ理由）。2026-09-28: このフィールドがRust側の
   * ペイロードには存在するのにWorker側でallowlist再構築時に見落とされており、
   * 送信されても黙って消えていた（`running_processes`追加時の監査で発覚）。 */
  keymap_learn: Record<string, unknown> | null;
  /** issue #165（hook_starved）用（2026-09-28追記）。`SCHEMA_VERSION`は
   * 上げていないため、旧クライアントが生成した報告にはこの2フィールドが
   * 存在しない（`retro_eval_stats`と同じ理由でoptionalとして読む）。他の
   * `attach_*`と違い既定オフのチェックボックスのため、実際に添付される
   * 報告は少ない見込み。 */
  attach_running_processes: boolean;
  running_processes: string[] | null;
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

/** GitHub側の実障害（`!response.ok`）を表す。`fetchLatestTagForLine`が`null`を
 * 返す「該当ラインのリリースが単に無い」ケースと区別するために使う
 * （opusコードレビュー指摘: 混同すると本来503であるべき障害が404として報告される）。 */
class UpstreamFetchError extends Error {}

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

  // `?current_version=` を送らない（旧）クライアントは従来通り全体の「latest」を見る。
  // 送ってくるが値がSemVerとして読めない場合も安全側に倒して同じ扱いにする。
  const line = requestedReleaseLine(new URL(request.url));
  const cacheKey = releaseCacheKey(line);

  // RATE_LIMIT_KV is named for report rate limits, but also stores the release cache by prefix.
  const cached = parseLatestReleaseCacheEntry(await env.RATE_LIMIT_KV.get(cacheKey));
  if (cached === null) {
    let refreshed: LatestReleaseCacheEntry | null;
    try {
      refreshed = await fetchAndCacheLatestRelease(env, line);
    } catch (error) {
      if (error instanceof UpstreamFetchError) {
        console.error("latest release fetch failed", error);
        return jsonResponse({ error: "upstream_unavailable" }, 503);
      }
      throw error;
    }
    if (refreshed === null) {
      return jsonResponse(
        { error: line === null ? "upstream_unavailable" : "no_release_for_line" },
        line === null ? 503 : 404
      );
    }
    return jsonResponse(latestReleaseResponse(refreshed, false), 200);
  }

  if (isLatestReleaseFresh(cached, new Date())) {
    return jsonResponse(latestReleaseResponse(cached, false), 200);
  }

  ctx.waitUntil(refreshStaleLatestRelease(env, line));
  return jsonResponse(latestReleaseResponse(cached, true), 200);
}

/** `line === null` は旧クライアント互換の「ライン区別なし・全体のlatest」モード。 */
function requestedReleaseLine(url: URL): ReleaseLine | null {
  const currentVersion = url.searchParams.get("current_version");
  if (currentVersion === null) {
    return null;
  }
  const parsed = parseSemver(currentVersion);
  return parsed === null ? null : releaseLine(parsed);
}

function releaseCacheKey(line: ReleaseLine | null): string {
  return line === null ? RELEASE_CACHE_KEY : `latest-release:line:${line}`;
}

function releaseRefreshingKey(line: ReleaseLine | null): string {
  return line === null ? RELEASE_REFRESHING_KEY : `latest-release:refreshing:${line}`;
}

async function refreshStaleLatestRelease(
  env: Env,
  line: ReleaseLine | null
): Promise<LatestReleaseCacheEntry | null> {
  const refreshingKey = releaseRefreshingKey(line);
  if (await env.RATE_LIMIT_KV.get(refreshingKey) !== null) {
    return null;
  }

  await env.RATE_LIMIT_KV.put(refreshingKey, "1", {
    expirationTtl: RELEASE_REFRESHING_TTL_SECONDS
  });
  try {
    return await fetchAndCacheLatestRelease(env, line);
  } catch (error) {
    console.error("latest release background refresh failed", error);
    return null;
  }
}

async function fetchAndCacheLatestRelease(
  env: Env,
  line: ReleaseLine | null
): Promise<LatestReleaseCacheEntry | null> {
  try {
    const tagName = await fetchLatestTagForLine(line);
    if (tagName === null) {
      return null;
    }

    const now = new Date().toISOString();
    const entry: LatestReleaseCacheEntry = {
      schema_version: 1,
      latest_version: normalizeReleaseVersion(tagName),
      checked_at: now,
      fetched_at: now
    };

    await env.RATE_LIMIT_KV.put(releaseCacheKey(line), JSON.stringify(entry), {
      expirationTtl: RELEASE_CACHE_EXPIRATION_TTL_SECONDS
    });
    return entry;
  } catch (error) {
    // GitHub側の実障害（UpstreamFetchError）は呼び出し元が404/503を区別できるよう
    // 伝播させる。それ以外（JSON parse失敗・KV書き込み失敗等）は従来通り握り潰してnull。
    if (error instanceof UpstreamFetchError) {
      throw error;
    }
    console.error("latest release refresh failed", error);
    return null;
  }
}

async function githubGet(url: string): Promise<Response> {
  return fetch(url, {
    method: "GET",
    headers: {
      "User-Agent": "awase-update-check-worker (+https://awase.cc)",
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28"
    }
  });
}

async function fetchLatestTagForLine(line: ReleaseLine | null): Promise<string | null> {
  if (line === null) {
    const response = await githubGet(GITHUB_LATEST_RELEASE_URL);
    if (!response.ok) {
      throw new UpstreamFetchError(`GitHub latest-release request failed: ${response.status}`);
    }
    const body: unknown = await response.json();
    return isRecord(body) && typeof body.tag_name === "string" ? body.tag_name : null;
  }

  const items: unknown[] = [];
  for (let page = 1; page <= GITHUB_RELEASES_MAX_PAGES; page += 1) {
    const response = await githubGet(`${GITHUB_RELEASES_LIST_URL}&page=${page}`);
    if (!response.ok) {
      throw new UpstreamFetchError(`GitHub releases list request failed: ${response.status}`);
    }
    const body: unknown = await response.json();
    if (!Array.isArray(body)) {
      break;
    }
    items.push(...body);
    if (body.length < GITHUB_RELEASES_PER_PAGE) {
      break; // 最終ページ
    }
  }
  return highestTagForLine(items, line);
}

/** `items` はGitHub `/releases` 一覧レスポンス（複数ページ分をマージ済み）。draft/
 * prereleaseは除外し、自ライン内でSemVer最大のtag_nameを返す（`/releases/latest`は
 * ライン区別できないため使わない）。 */
function highestTagForLine(items: unknown[], line: ReleaseLine): string | null {
  let best: { tag: string; version: Semver } | null = null;
  for (const item of items) {
    if (!isRecord(item) || item.draft === true || item.prerelease === true) {
      continue;
    }
    if (typeof item.tag_name !== "string") {
      continue;
    }
    const version = parseSemver(normalizeReleaseVersion(item.tag_name));
    if (version === null || releaseLine(version) !== line) {
      continue;
    }
    if (best === null || compareSemver(version, best.version) > 0) {
      best = { tag: item.tag_name, version };
    }
  }
  return best?.tag ?? null;
}

export function parseSemver(version: string): Semver | null {
  const match = /^(\d+)\.(\d+)\.(\d+)/.exec(version);
  if (match?.[1] === undefined || match[2] === undefined || match[3] === undefined) {
    return null;
  }
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

function compareSemver(a: Semver, b: Semver): number {
  for (let i = 0; i < 3; i += 1) {
    const diff = (a[i] ?? 0) - (b[i] ?? 0);
    if (diff !== 0) {
      return diff;
    }
  }
  return 0;
}

export function releaseLine(version: Semver): ReleaseLine {
  return compareSemver(version, V2_LINE_MIN_VERSION) >= 0 ? "v2" : "v1";
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

  if (
    typeof value.schema_version !== "number" ||
    !SUPPORTED_SCHEMA_VERSIONS.includes(value.schema_version)
  ) {
    throw new HttpError(400, "unsupported_schema_version");
  }
  const schemaVersion = value.schema_version as 3 | 4;

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
  // ADR-222: schema_version 4 の gzip フィールド。3 までのクライアントは送らない（null 扱い）。
  const logExcerptGz = optionalNullableGzipBase64(value, "log_excerpt_gz");
  const appLogExcerptGz = optionalNullableGzipBase64(value, "app_log_excerpt_gz");
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
  // ADR-148: 上記2フィールドと同じ理由でoptionalとして読む。
  const attachImeKeymap = optionalBoolean(value, "attach_ime_keymap");
  const gjiKeymap = optionalNullableRecord(value, "gji_keymap");
  const msimeKeyAssignment = optionalNullableRecord(value, "msime_key_assignment");
  // ADR-148 Phase 2: attach_ime_keymap に相乗り。上記2フィールドと同じ理由でoptional。
  const legacyMsimeKeymap = optionalNullableRecord(value, "legacy_msime_keymap");
  // ADR196-T2 決定1e後半: attach_ime_keymap に相乗り。上記と同じ理由でoptional。
  const keymapLearn = optionalNullableRecord(value, "keymap_learn");
  // issue #165（hook_starved）用。上記と同じ理由でoptionalとして読む。
  const attachRunningProcesses = optionalBoolean(value, "attach_running_processes");
  const runningProcesses = optionalNullableStringArray(value, "running_processes");
  const reportedAt = requiredString(value, "reported_at");
  if (Number.isNaN(Date.parse(reportedAt))) {
    throw new HttpError(400, "reported_at_must_be_rfc3339");
  }

  if (schemaVersion === 4 && (logExcerpt !== null || appLogExcerpt !== null)) {
    // 4 のクライアントは非圧縮フィールドを使わない。両方ある報告は曖昧なので拒否する。
    throw new HttpError(400, "legacy_log_fields_not_allowed_in_schema_4");
  }
  if (schemaVersion === 3 && (logExcerptGz !== null || appLogExcerptGz !== null)) {
    throw new HttpError(400, "gz_log_fields_require_schema_4");
  }
  if (!attachLog && (logExcerptGz !== null || appLogExcerptGz !== null)) {
    throw new HttpError(400, "log_excerpt_gz_requires_attach_log");
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
  // ADR-148: attach_ime_keymap は1フラグだが紐づくデータはgji_keymap/
  // msime_key_assignmentの2オブジェクトなので、整合性チェックも2本必要
  // （1フラグ1オブジェクトの他フィールドとカーディナリティが異なる）。
  if (!attachImeKeymap && gjiKeymap !== null) {
    throw new HttpError(400, "gji_keymap_requires_attach_ime_keymap");
  }
  if (!attachImeKeymap && msimeKeyAssignment !== null) {
    throw new HttpError(400, "msime_key_assignment_requires_attach_ime_keymap");
  }
  if (!attachImeKeymap && legacyMsimeKeymap !== null) {
    throw new HttpError(400, "legacy_msime_keymap_requires_attach_ime_keymap");
  }
  if (!attachImeKeymap && keymapLearn !== null) {
    throw new HttpError(400, "keymap_learn_requires_attach_ime_keymap");
  }
  if (!attachRunningProcesses && runningProcesses !== null) {
    throw new HttpError(400, "running_processes_requires_attach_running_processes");
  }

  return {
    schema_version: schemaVersion,
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
    log_excerpt_gz: logExcerptGz,
    app_log_excerpt: appLogExcerpt,
    app_log_excerpt_gz: appLogExcerptGz,
    attach_state_snapshot: attachStateSnapshot,
    state_snapshot: stateSnapshot,
    attach_config: attachConfig,
    config_toml: configToml,
    attach_layout: attachLayout,
    layout_yab: layoutYab,
    attach_retro_eval_stats: attachRetroEvalStats,
    retro_eval_stats: retroEvalStats,
    attach_ime_keymap: attachImeKeymap,
    gji_keymap: gjiKeymap,
    msime_key_assignment: msimeKeyAssignment,
    legacy_msime_keymap: legacyMsimeKeymap,
    keymap_learn: keymapLearn,
    attach_running_processes: attachRunningProcesses,
    running_processes: runningProcesses,
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

/** ADR-222: gzip して base64 にした文字列（またはフィールド無し / null）。Worker は解凍しない
 * （無料プランの CPU 時間と解凍爆弾を避ける）ので、形式だけを安く検証する: 長さの上限、
 * base64 の文字種と 4 の倍数の長さ、gzip の先頭バイト（1f 8b 08 は base64 で `H4sI`）。
 * 中身の検証・展開は、メンテナの手元（`bug-report-fetch`）で展開後サイズに上限を付けて行う。 */
function optionalNullableGzipBase64(
  value: Record<string, unknown>,
  field: string
): string | null {
  const fieldValue = value[field];
  if (fieldValue === undefined || fieldValue === null) {
    return null;
  }
  if (typeof fieldValue !== "string") {
    throw new HttpError(400, `${field}_invalid`);
  }
  if (fieldValue.length > MAX_LOG_GZ_BASE64_CHARS) {
    throw new HttpError(400, `${field}_too_large`);
  }
  if (
    fieldValue.length % 4 !== 0 ||
    !fieldValue.startsWith("H4sI") ||
    !/^[A-Za-z0-9+/]*={0,2}$/.test(fieldValue)
  ) {
    throw new HttpError(400, `${field}_invalid`);
  }
  return fieldValue;
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

/**
 * `requiredStringArray` と異なり、フィールド自体が存在しない（`undefined`）
 * 場合、または`null`の場合は`null`として受理する。issue #165(hook_starved)
 * 用の`running_processes`で導入 — `optionalNullableRecord`と同じ理由
 * （旧クライアント・attach_running_processes=falseの報告を拒否しない）。
 */
function optionalNullableStringArray(
  value: Record<string, unknown>,
  field: string
): string[] | null {
  const fieldValue = value[field];
  if (fieldValue === undefined || fieldValue === null) {
    return null;
  }
  if (Array.isArray(fieldValue) && fieldValue.every((item) => typeof item === "string")) {
    return fieldValue;
  }
  throw new HttpError(400, `${field}_invalid`);
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
