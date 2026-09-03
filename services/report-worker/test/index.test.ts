import {
  assertBodySizeFromContentLength,
  handleRequest,
  HttpError,
  incrementDailyRateLimit,
  MAX_BODY_BYTES,
  parseAndValidatePayload,
  RELEASE_CACHE_KEY
} from "../src/index";

const validPayload = {
  schema_version: 3,
  app_version: "1.15.0",
  os_version: "Windows 11 Build 22631",
  ime_kind: "Gji",
  ime_product_name: "Google 日本語入力",
  keyboard_model: "Jis",
  windows_keyboard_layout: "LANGID=0x0411 (Japanese=true)",
  competing_software: ["やまぶき"],
  symptom_category: "WrongCharacterOutput",
  description: "変換が意図通りに動きません",
  attach_log: true,
  log_excerpt: "journal excerpt",
  app_log_excerpt: "app log excerpt",
  attach_state_snapshot: false,
  state_snapshot: null,
  attach_config: false,
  config_toml: null,
  attach_layout: false,
  layout_yab: null,
  attach_retro_eval_stats: false,
  retro_eval_stats: null,
  reported_at: "2026-08-19T12:34:56Z"
};

class MemoryKv {
  values = new Map<string, string>();
  puts: Array<{ key: string; value: string; options?: { expirationTtl?: number } }> = [];

  async get(key: string): Promise<string | null> {
    return this.values.get(key) ?? null;
  }

  async put(
    key: string,
    value: string,
    options?: { expirationTtl?: number }
  ): Promise<void> {
    this.values.set(key, value);
    if (options === undefined) {
      this.puts.push({ key, value });
    } else {
      this.puts.push({ key, value, options });
    }
  }
}

class MemoryBucket {
  puts: Array<{ key: string; value: string }> = [];

  async put(key: string, value: string): Promise<void> {
    this.puts.push({ key, value });
  }
}

describe("payload validation", () => {
  it("accepts the documented payload shape", () => {
    expect(parseAndValidatePayload(JSON.stringify(validPayload))).toEqual(validPayload);
  });

  it("accepts null ime product name and no competing software", () => {
    const payload = {
      ...validPayload,
      ime_product_name: null,
      competing_software: []
    };

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual(payload);
  });

  it("accepts payloads without app_log_excerpt (pre-BUG-34 clients) and normalizes to null", () => {
    const { app_log_excerpt: _appLogExcerpt, ...payload } = validPayload;

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual({
      ...payload,
      app_log_excerpt: null
    });
  });

  it("accepts an explicit null app_log_excerpt", () => {
    const payload = { ...validPayload, app_log_excerpt: null };

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual(payload);
  });

  it("rejects a non-string, non-null app_log_excerpt", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        app_log_excerpt: 42
      })),
      400,
      "app_log_excerpt_invalid"
    );
  });

  it("rejects app_log_excerpt unless attach_log is set", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_log: false,
        log_excerpt: null,
        app_log_excerpt: "app log excerpt"
      })),
      400,
      "app_log_excerpt_requires_attach_log"
    );
  });

  it("rejects invalid JSON", () => {
    expectHttpError(() => parseAndValidatePayload("{"), 400, "invalid_json");
  });

  it("rejects missing description fields", () => {
    const { description: _description, ...payload } = validPayload;

    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify(payload)),
      400,
      "description_required"
    );
  });

  it("rejects unsupported schema versions", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({ ...validPayload, schema_version: 2 })),
      400,
      "unsupported_schema_version"
    );
  });

  it("rejects missing symptom categories", () => {
    const { symptom_category: _symptomCategory, ...payload } = validPayload;

    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify(payload)),
      400,
      "invalid_symptom_category"
    );
  });

  it("rejects invalid symptom categories", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        symptom_category: "KeyboardLag"
      })),
      400,
      "invalid_symptom_category"
    );
  });

  it("rejects invalid keyboard models", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        keyboard_model: "Jp"
      })),
      400,
      "keyboard_model_required"
    );
  });

  it("rejects empty Windows keyboard layout strings", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        windows_keyboard_layout: ""
      })),
      400,
      "windows_keyboard_layout_required"
    );
  });

  it("rejects non-string competing software entries", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        competing_software: ["やまぶき", 42]
      })),
      400,
      "competing_software_required"
    );
  });

  it("accepts an empty description for non-other symptom categories", () => {
    expect(parseAndValidatePayload(JSON.stringify({
      ...validPayload,
      description: ""
    }))).toEqual({
      ...validPayload,
      description: ""
    });
  });

  it("rejects an empty description for other symptom categories", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        symptom_category: "Other",
        description: " \n\t"
      })),
      400,
      "description_required_for_other_category"
    );
  });

  it("accepts a description for other symptom categories", () => {
    const payload = {
      ...validPayload,
      symptom_category: "Other",
      description: "一覧にない症状です"
    };

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual(payload);
  });

  it("rejects a state snapshot unless explicitly attached", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_state_snapshot: false,
        state_snapshot: { desired_open: true }
      })),
      400,
      "state_snapshot_requires_attach_state_snapshot"
    );
  });

  it("rejects config TOML unless explicitly attached", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_config: false,
        config_toml: "[general]\n"
      })),
      400,
      "config_toml_requires_attach_config"
    );
  });

  it("rejects layout YAB unless explicitly attached", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_layout: false,
        layout_yab: "# layout\n"
      })),
      400,
      "layout_yab_requires_attach_layout"
    );
  });

  it("accepts an explicitly attached state snapshot object", () => {
    const payload = {
      ...validPayload,
      attach_state_snapshot: true,
      state_snapshot: {
        desired_open: true,
        input_mode: "ObservedRomaji",
        nested: {
          app_kind: "Editor"
        }
      }
    };

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual(payload);
  });

  // ADR-120 決定0a-report: SCHEMA_VERSION は上げていないため、この変更より前の
  // クライアント（retro_eval_stats 関連フィールドを一切送らない v3相当の
  // ペイロード）が引き続き200で受理されることを固定する（この変更の核心）。
  it("accepts payloads without retro_eval_stats fields (pre-ADR-120 clients) and normalizes to false/null", () => {
    const {
      attach_retro_eval_stats: _attachRetroEvalStats,
      retro_eval_stats: _retroEvalStats,
      ...payload
    } = validPayload;

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual({
      ...payload,
      attach_retro_eval_stats: false,
      retro_eval_stats: null
    });
  });

  it("accepts an explicit null retro_eval_stats", () => {
    const payload = {
      ...validPayload,
      attach_retro_eval_stats: false,
      retro_eval_stats: null
    };

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual(payload);
  });

  it("rejects a non-boolean attach_retro_eval_stats", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_retro_eval_stats: "yes"
      })),
      400,
      "attach_retro_eval_stats_invalid"
    );
  });

  it("rejects a non-object, non-null retro_eval_stats", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_retro_eval_stats: true,
        retro_eval_stats: 42
      })),
      400,
      "retro_eval_stats_invalid"
    );
  });

  it("rejects retro_eval_stats unless explicitly attached", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_retro_eval_stats: false,
        retro_eval_stats: { three_key_total: 10 }
      })),
      400,
      "retro_eval_stats_requires_attach_retro_eval_stats"
    );
  });

  it("accepts an explicitly attached retro_eval_stats object", () => {
    const payload = {
      ...validPayload,
      attach_retro_eval_stats: true,
      retro_eval_stats: {
        three_key_total: 10,
        phase2_reached: 3,
        followup_elapsed_ms_histogram: [1, 2, 3, 4, 5, 6, 7]
      }
    };

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual(payload);
  });
});

describe("request validation", () => {
  it("returns 400 when symptom_category is missing", async () => {
    const { symptom_category: _symptomCategory, ...payload } = validPayload;

    await expectPostStatus(payload, 400, "invalid_symptom_category");
  });

  it("returns 400 when symptom_category is invalid", async () => {
    await expectPostStatus(
      { ...validPayload, symptom_category: "KeyboardLag" },
      400,
      "invalid_symptom_category"
    );
  });

  it("returns 201 when description is empty for a non-other category", async () => {
    await expectPostStatus({ ...validPayload, description: "" }, 201);
  });

  it("returns 400 when description is empty for other category", async () => {
    await expectPostStatus(
      { ...validPayload, symptom_category: "Other", description: "" },
      400,
      "description_required_for_other_category"
    );
  });

  it("returns 201 when description is present for other category", async () => {
    await expectPostStatus(
      { ...validPayload, symptom_category: "Other", description: "一覧にない症状です" },
      201
    );
  });
});

describe("request size validation", () => {
  it("rejects Content-Length values over 512 KiB", () => {
    expectHttpError(
      () => assertBodySizeFromContentLength(String(MAX_BODY_BYTES + 1), MAX_BODY_BYTES),
      413,
      "request_body_too_large"
    );
  });
});

describe("rate limiting", () => {
  it("increments a daily counter and stores it with a day-bounded TTL", async () => {
    const kv = new MemoryKv();
    const at = new Date("2026-08-19T12:00:00Z");

    const first = await incrementDailyRateLimit(kv, "203.0.113.10", at, 2);
    const second = await incrementDailyRateLimit(kv, "203.0.113.10", at, 2);

    expect(first.allowed).toBe(true);
    expect(first.count).toBe(1);
    expect(second.allowed).toBe(true);
    expect(second.count).toBe(2);
    expect(first.key).toBe(second.key);
    expect(first.key).not.toContain("203.0.113.10");
    expect(kv.puts.at(-1)?.options?.expirationTtl).toBe(43200);
  });

  it("blocks requests over the daily limit without writing a new counter", async () => {
    const kv = new MemoryKv();
    const at = new Date("2026-08-19T12:00:00Z");

    await incrementDailyRateLimit(kv, "203.0.113.10", at, 1);
    const blocked = await incrementDailyRateLimit(kv, "203.0.113.10", at, 1);

    expect(blocked.allowed).toBe(false);
    expect(blocked.count).toBe(2);
    expect(kv.puts).toHaveLength(1);
  });
});

describe("latest release endpoint", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("fetches GitHub synchronously on an empty cache and stores the result", async () => {
    const kv = new MemoryKv();
    const pending: Promise<unknown>[] = [];
    const fetchMock = mockGithubLatestRelease("v1.19.0");

    const response = await handleRequest(
      latestReleaseRequest(),
      latestReleaseEnv(kv),
      fakeCtx(pending)
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({
      schema_version: 1,
      latest_version: "1.19.0",
      checked_at: expect.any(String),
      stale: false
    });
    expect(kv.values.has(RELEASE_CACHE_KEY)).toBe(true);
    expect(kv.puts.find((put) => put.key === RELEASE_CACHE_KEY)?.options?.expirationTtl).toBe(
      86400
    );
    expect(pending).toHaveLength(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("returns 503 when the cache is empty and GitHub is unavailable", async () => {
    mockGithubResponse(new Response("rate limited", { status: 403 }));

    const response = await handleRequest(
      latestReleaseRequest(),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    expect(response.status).toBe(503);
    await expect(response.json()).resolves.toEqual({ error: "upstream_unavailable" });
  });

  it("serves a fresh cached release without calling GitHub", async () => {
    const kv = new MemoryKv();
    kv.values.set(
      RELEASE_CACHE_KEY,
      JSON.stringify(cacheEntry("1.18.0", new Date().toISOString()))
    );
    const fetchMock = mockGithubLatestRelease("v1.19.0");

    const response = await handleRequest(
      latestReleaseRequest(),
      latestReleaseEnv(kv),
      fakeCtx([])
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({
      schema_version: 1,
      latest_version: "1.18.0",
      checked_at: expect.any(String),
      stale: false
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("returns stale cache immediately and refreshes it through waitUntil", async () => {
    const kv = new MemoryKv();
    kv.values.set(
      RELEASE_CACHE_KEY,
      JSON.stringify(cacheEntry("1.18.0", "2000-01-01T00:00:00Z"))
    );
    const pending: Promise<unknown>[] = [];
    const github = mockDeferredGithubLatestRelease();

    const response = await handleRequest(
      latestReleaseRequest(),
      latestReleaseEnv(kv),
      fakeCtx(pending)
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toEqual({
      schema_version: 1,
      latest_version: "1.18.0",
      checked_at: "2000-01-01T00:00:00Z",
      stale: true
    });
    expect(JSON.parse(kv.values.get(RELEASE_CACHE_KEY) ?? "{}")).toMatchObject({
      latest_version: "1.18.0"
    });

    github.resolve("v1.19.0");
    await Promise.all(pending);
    expect(JSON.parse(kv.values.get(RELEASE_CACHE_KEY) ?? "{}")).toMatchObject({
      latest_version: "1.19.0"
    });
  });

  it("keeps serving stale cache when a background refresh fails", async () => {
    const kv = new MemoryKv();
    kv.values.set(
      RELEASE_CACHE_KEY,
      JSON.stringify(cacheEntry("1.18.0", "2000-01-01T00:00:00Z"))
    );
    const pending: Promise<unknown>[] = [];
    mockGithubResponse(new Response("rate limited", { status: 403 }));

    const response = await handleRequest(
      latestReleaseRequest(),
      latestReleaseEnv(kv),
      fakeCtx(pending)
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toMatchObject({
      latest_version: "1.18.0",
      stale: true
    });
    await Promise.all(pending);
    expect(JSON.parse(kv.values.get(RELEASE_CACHE_KEY) ?? "{}")).toMatchObject({
      latest_version: "1.18.0"
    });
  });

  it("does not call GitHub for stale cache when a refresh is already in progress", async () => {
    const kv = new MemoryKv();
    kv.values.set(
      RELEASE_CACHE_KEY,
      JSON.stringify(cacheEntry("1.18.0", "2000-01-01T00:00:00Z"))
    );
    kv.values.set("latest-release:refreshing", "1");
    const pending: Promise<unknown>[] = [];
    const fetchMock = mockGithubLatestRelease("v1.19.0");

    const response = await handleRequest(
      latestReleaseRequest(),
      latestReleaseEnv(kv),
      fakeCtx(pending)
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toMatchObject({
      latest_version: "1.18.0",
      stale: true
    });
    await Promise.all(pending);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("writes the refreshing marker with a 60 second TTL on the stale path", async () => {
    const kv = new MemoryKv();
    kv.values.set(
      RELEASE_CACHE_KEY,
      JSON.stringify(cacheEntry("1.18.0", "2000-01-01T00:00:00Z"))
    );
    const pending: Promise<unknown>[] = [];
    mockGithubLatestRelease("v1.19.0");

    await handleRequest(latestReleaseRequest(), latestReleaseEnv(kv), fakeCtx(pending));
    await Promise.all(pending);

    expect(kv.puts).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          key: "latest-release:refreshing",
          options: { expirationTtl: 60 }
        })
      ])
    );
  });

  it("sends the required GitHub User-Agent header", async () => {
    const fetchMock = mockGithubLatestRelease("v1.19.0");

    await handleRequest(latestReleaseRequest(), latestReleaseEnv(new MemoryKv()), fakeCtx([]));

    const firstCall = fetchMock.mock.calls[0];
    if (firstCall === undefined) {
      throw new Error("expected GitHub fetch");
    }
    const init = firstCall[1] as RequestInit | undefined;
    expect(new Headers(init?.headers).get("User-Agent")).toBe(
      "awase-update-check-worker (+https://awase.cc)"
    );
  });

  it("rejects unsupported methods with the GET and HEAD Allow header", async () => {
    const response = await handleRequest(
      latestReleaseRequest({ method: "POST" }),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    expect(response.status).toBe(405);
    expect(response.headers.get("Allow")).toBe("GET, HEAD");
  });

  it("accepts HEAD with the same headers as GET", async () => {
    const kv = new MemoryKv();
    kv.values.set(
      RELEASE_CACHE_KEY,
      JSON.stringify(cacheEntry("1.18.0", new Date().toISOString()))
    );

    const response = await handleRequest(
      latestReleaseRequest({ method: "HEAD" }),
      latestReleaseEnv(kv),
      fakeCtx([])
    );

    expect(response.status).toBe(200);
    expect(response.headers.get("Content-Type")).toBe("application/json; charset=utf-8");
  });

  it("does not include client-unused release URL fields", async () => {
    mockGithubLatestRelease("v1.19.0");

    const response = await handleRequest(
      latestReleaseRequest(),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );
    const body = await response.json();

    expect(body).not.toHaveProperty("download_url");
    expect(body).not.toHaveProperty("tag");
    expect(body).not.toHaveProperty("released_at");
    expect(body).not.toHaveProperty("release_url");
  });

  it("keeps report intake routing behavior unchanged", async () => {
    const env = latestReleaseEnv(new MemoryKv());

    const methodResponse = await handleRequest(
      new Request("https://report.awase.cc/v1/reports"),
      env,
      fakeCtx([])
    );
    const notFoundResponse = await handleRequest(
      new Request("https://report.awase.cc/v1/unknown"),
      env,
      fakeCtx([])
    );

    expect(methodResponse.status).toBe(405);
    expect(methodResponse.headers.get("Allow")).toBe("POST");
    expect(notFoundResponse.status).toBe(404);
  });
});

function expectHttpError(action: () => unknown, status: number, message: string): void {
  try {
    action();
  } catch (error) {
    expect(error).toBeInstanceOf(HttpError);
    expect((error as HttpError).status).toBe(status);
    expect((error as HttpError).message).toBe(message);
    return;
  }

  throw new Error("expected HttpError");
}

function latestReleaseRequest(init?: RequestInit): Request {
  return new Request("https://report.awase.cc/v1/latest-release", init);
}

function latestReleaseEnv(kv: MemoryKv): {
  REPORT_BUCKET: R2Bucket;
  RATE_LIMIT_KV: KVNamespace;
} {
  return {
    REPORT_BUCKET: new MemoryBucket() as unknown as R2Bucket,
    RATE_LIMIT_KV: kv as unknown as KVNamespace
  };
}

function fakeCtx(pending: Promise<unknown>[]): ExecutionContext {
  return {
    waitUntil(promise: Promise<unknown>) {
      pending.push(promise);
    },
    passThroughOnException() {}
  } as ExecutionContext;
}

function cacheEntry(latestVersion: string, fetchedAt: string): {
  schema_version: 1;
  latest_version: string;
  checked_at: string;
  fetched_at: string;
} {
  return {
    schema_version: 1,
    latest_version: latestVersion,
    checked_at: fetchedAt,
    fetched_at: fetchedAt
  };
}

function mockGithubLatestRelease(tagName: string) {
  return mockGithubResponse(Response.json({ tag_name: tagName }));
}

function mockDeferredGithubLatestRelease(): {
  resolve: (tagName: string) => void;
} {
  let resolveResponse: ((response: Response) => void) | undefined;
  const responsePromise = new Promise<Response>((resolve) => {
    resolveResponse = resolve;
  });
  const fetchMock = vi.fn(
    async (_input: RequestInfo | URL, _init?: RequestInit): Promise<Response> => responsePromise
  );
  vi.stubGlobal("fetch", fetchMock);

  return {
    resolve(tagName: string) {
      if (resolveResponse === undefined) {
        throw new Error("deferred GitHub mock was not initialized");
      }
      resolveResponse(Response.json({ tag_name: tagName }));
    }
  };
}

function mockGithubResponse(response: Response) {
  const fetchMock = vi.fn(
    async (_input: RequestInfo | URL, _init?: RequestInit): Promise<Response> => response
  );
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

async function expectPostStatus(
  payload: unknown,
  status: number,
  error?: string
): Promise<void> {
  const response = await handleRequest(
    new Request("https://report.awase.cc/v1/reports", {
      method: "POST",
      headers: {
        "CF-Connecting-IP": "203.0.113.10",
        "Content-Type": "application/json"
      },
      body: JSON.stringify(payload)
    }),
    {
      REPORT_BUCKET: new MemoryBucket() as unknown as R2Bucket,
      RATE_LIMIT_KV: new MemoryKv() as unknown as KVNamespace
    },
    fakeCtx([])
  );
  expect(response.status).toBe(status);
  if (error !== undefined) {
    await expect(response.json()).resolves.toEqual({ error });
  }
}
