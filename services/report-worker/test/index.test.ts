import {
  assertBodySizeFromContentLength,
  handleRequest,
  HttpError,
  incrementDailyRateLimit,
  MAX_BODY_BYTES,
  parseAndValidatePayload
} from "../src/index";

const validPayload = {
  schema_version: 2,
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
      () => parseAndValidatePayload(JSON.stringify({ ...validPayload, schema_version: 1 })),
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
    }
  );
  expect(response.status).toBe(status);
  if (error !== undefined) {
    await expect(response.json()).resolves.toEqual({ error });
  }
}
