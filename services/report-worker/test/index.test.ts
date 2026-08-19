import {
  assertBodySizeFromContentLength,
  HttpError,
  incrementDailyRateLimit,
  MAX_BODY_BYTES,
  parseAndValidatePayload
} from "../src/index";

const validPayload = {
  schema_version: 1,
  app_version: "1.15.0",
  os_version: "Windows 11 Build 22631",
  ime_kind: "Gji",
  description: "変換が意図通りに動きません",
  attach_log: true,
  log_excerpt: "journal excerpt",
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

describe("payload validation", () => {
  it("accepts the documented payload shape", () => {
    expect(parseAndValidatePayload(JSON.stringify(validPayload))).toEqual(validPayload);
  });

  it("rejects invalid JSON", () => {
    expectHttpError(() => parseAndValidatePayload("{"), 400, "invalid_json");
  });

  it("rejects missing required fields", () => {
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
