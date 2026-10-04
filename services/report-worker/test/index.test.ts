import {
  assertBodySizeFromContentLength,
  handleRequest,
  HttpError,
  incrementDailyRateLimit,
  MAX_BODY_BYTES,
  MAX_LOG_GZ_BASE64_CHARS,
  parseAndValidatePayload,
  parseSemver,
  RELEASE_CACHE_KEY,
  releaseLine
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
  // ADR-222: schema_version 3（旧クライアント）はこの 2 フィールドを送らない。受理後は
  // null に正規化されるので、`toEqual` の比較を単純にするため、ここでは null を持たせる。
  log_excerpt_gz: null,
  app_log_excerpt: "app log excerpt",
  app_log_excerpt_gz: null,
  attach_state_snapshot: false,
  state_snapshot: null,
  attach_config: false,
  config_toml: null,
  attach_layout: false,
  layout_yab: null,
  attach_retro_eval_stats: false,
  retro_eval_stats: null,
  attach_ime_keymap: false,
  gji_keymap: null,
  msime_key_assignment: null,
  legacy_msime_keymap: null,
  keymap_learn: null,
  attach_running_processes: false,
  running_processes: null,
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

// Worker は解凍しないので、フィクスチャは固定の gzip(base64) 文字列でよい（生成: Python の
// gzip.compress(text, mtime=0) を base64）。`H4sI` は gzip の先頭 1f 8b 08 の base64 表現。
const GZIP_JOURNAL = "H4sIAAAAAAAC/4uuVipOLVSyMtRRSs0rKapUsqpWKqksSFWyUvJOrfTMKygtUaqtjQUAuTk7CScAAAA="; // [{"seq":1,"entry":{"type":"KeyInput"}}]
const GZIP_APP_LOG = "H4sIAAAAAAAC/zMyMDLTNTTQNTAJMTCyMjKxMjDSMzQ0MDa0jFLw9HPzV0gsTyxOVSguSSwqSU3hAgBXnuEzLwAAAA=="; // 2026-10-04T02:24:02.110319Z INFO awase started
const GZIP_EMPTY_ARRAY = "H4sIAAAAAAAC/4uOBQApu0wNAgAAAA=="; // []

// ADR-222: schema_version 4（gzip + base64 のログ）。新クライアントは非圧縮の
// log_excerpt / app_log_excerpt を null にして、`_gz` に入れて送る。
const validPayloadV4 = {
  ...validPayload,
  schema_version: 4,
  log_excerpt: null,
  log_excerpt_gz: GZIP_JOURNAL,
  app_log_excerpt: null,
  app_log_excerpt_gz: GZIP_APP_LOG
};

describe("schema_version 4 (gzip logs, ADR-222)", () => {
  it("accepts a schema_version 4 payload and keeps the gzip fields as they are", () => {
    expect(parseAndValidatePayload(JSON.stringify(validPayloadV4))).toEqual(validPayloadV4);
  });

  it("still accepts schema_version 3 payloads (old clients, including the v1 maintenance line)", () => {
    expect(parseAndValidatePayload(JSON.stringify(validPayload))).toEqual(validPayload);
  });

  it("rejects schema versions other than 3 and 4", () => {
    for (const schema_version of [2, 5, "4", null]) {
      expect(() =>
        parseAndValidatePayload(JSON.stringify({ ...validPayloadV4, schema_version }))
      ).toThrowError(expect.objectContaining({ status: 400, message: "unsupported_schema_version" }));
    }
  });

  it("rejects the legacy plain-text log fields in schema_version 4", () => {
    // 古い Worker が知らないフィールドを黙って捨てて 201 を返す事故の対になる検証:
    // 4 のクライアントは非圧縮フィールドを使わない（両方ある報告は曖昧なので拒否する）。
    for (const field of ["log_excerpt", "app_log_excerpt"]) {
      expect(() =>
        parseAndValidatePayload(JSON.stringify({ ...validPayloadV4, [field]: "plain" }))
      ).toThrowError(
        expect.objectContaining({ status: 400, message: "legacy_log_fields_not_allowed_in_schema_4" })
      );
    }
  });

  it("rejects gzip fields in schema_version 3", () => {
    expect(() =>
      parseAndValidatePayload(
        JSON.stringify({ ...validPayload, log_excerpt_gz: GZIP_EMPTY_ARRAY })
      )
    ).toThrowError(expect.objectContaining({ status: 400, message: "gz_log_fields_require_schema_4" }));
  });

  it("rejects gzip fields unless attach_log is set", () => {
    expect(() =>
      parseAndValidatePayload(JSON.stringify({ ...validPayloadV4, attach_log: false }))
    ).toThrowError(expect.objectContaining({ status: 400, message: "log_excerpt_gz_requires_attach_log" }));
  });

  it("accepts null or absent gzip fields in schema_version 4 (attach_log without logs)", () => {
    const { log_excerpt_gz: _a, app_log_excerpt_gz: _b, ...rest } = validPayloadV4;
    expect(parseAndValidatePayload(JSON.stringify(rest))).toEqual({
      ...rest,
      log_excerpt_gz: null,
      app_log_excerpt_gz: null
    });
  });

  it("rejects gzip fields that are not valid base64 of a gzip stream", () => {
    const bad: Array<[unknown, string]> = [
      ["not base64!!", "log_excerpt_gz_invalid"],
      // 長さが 4 の倍数でない。
      ["H4sIAAA", "log_excerpt_gz_invalid"],
      // base64 としては正しいが gzip ではない（先頭が H4sI でない）。
      [Buffer.from("plain text, not gzip").toString("base64"), "log_excerpt_gz_invalid"],
      [42, "log_excerpt_gz_invalid"]
    ];
    for (const [value, code] of bad) {
      expect(() =>
        parseAndValidatePayload(JSON.stringify({ ...validPayloadV4, log_excerpt_gz: value }))
      ).toThrowError(expect.objectContaining({ status: 400, message: code }));
    }
  });

  it("checks the gzip field cheaply: head characters, length, magic and padding", () => {
    const fillerOk = "H4sI" + "A".repeat(4096 + 4096) + "AAAA";
    expect(fillerOk.length % 4).toBe(0);
    const accept = (value: string): void => {
      expect(
        parseAndValidatePayload(JSON.stringify({ ...validPayloadV4, log_excerpt_gz: value }))
          .log_excerpt_gz
      ).toBe(value);
    };
    const reject = (value: string): void => {
      expect(() =>
        parseAndValidatePayload(JSON.stringify({ ...validPayloadV4, log_excerpt_gz: value }))
      ).toThrowError(expect.objectContaining({ message: "log_excerpt_gz_invalid" }));
    };
    accept("H4sIAAAA");
    accept("H4sIAAA=");
    accept("H4sIAA==");
    accept(fillerOk);
    // 不正な末尾（パディング）。
    reject("H4sIAAAA====");
    reject("H4sIAA=A");
    // 先頭 4KiB 以内の不正な文字は弾く。
    reject("H4sI" + "A".repeat(100) + "!!!!" + "A".repeat(100));
    // 長さが短すぎる・4 の倍数でない。
    reject("H4sI");
    reject("H4sIAAAAA");
  });

  it("does not scan the whole gzip field (CPU on Workers Free); a bad char past the head is accepted", () => {
    // 全文の文字種は見ない（保存するだけ。壊れていれば調査側の b64decode(validate=True) が弾く）。
    // この振る舞いは、本体 1.75MiB で全文の正規表現が約 4ms（Free の CPU 10ms の 4 割）かかるための
    // 意図的な割り切りなので、テストで固定して、うっかり全文検証に戻さないようにする。
    const value = "H4sI" + "A".repeat(8192) + "!" + "A".repeat(3) + "AAAA";
    expect(value.length % 4).toBe(0);
    expect(
      parseAndValidatePayload(JSON.stringify({ ...validPayloadV4, log_excerpt_gz: value }))
        .log_excerpt_gz
    ).toBe(value);
  });

  it("rejects an oversized gzip field before inspecting it further", () => {
    const huge = "H4sI" + "A".repeat(MAX_LOG_GZ_BASE64_CHARS);
    expect(() =>
      parseAndValidatePayload(JSON.stringify({ ...validPayloadV4, app_log_excerpt_gz: huge }))
    ).toThrowError(expect.objectContaining({ status: 400, message: "app_log_excerpt_gz_too_large" }));
  });

  it("has a 2MiB body limit large enough for ten minutes of gzipped logs", () => {
    expect(MAX_BODY_BYTES).toBe(2 * 1024 * 1024);
    // 1 本の上限は、本体上限から他の項目の余裕（256KiB）を引いた値。2 本の合計は本体上限
    // （`readBodyWithLimit`）が別に抑える。
    expect(MAX_LOG_GZ_BASE64_CHARS).toBe(MAX_BODY_BYTES - 256 * 1024);
  });
});

describe("CPU cost of the largest accepted report (ADR-222 deploy check)", () => {
  // Workers Free プランの CPU 時間は 1 リクエスト 10ms。ADR-095 は「無料枠でカード登録なし」を
  // 前提に Cloudflare を選んでおり、このアカウントは Free の可能性が高い（API ではプランを
  // 確認できなかった）。本体上限 2MiB いっぱいの報告で、I/O を除く CPU 側の処理
  // （本文の復号・JSON 解析・検証・R2 保存用の直列化。整形なし = ハンドラと同じ）がどれだけ
  // かかるかを、CI のログに出す。最適化前（全文の正規表現 + 整形つき直列化）は本体 1.75MiB で
  // first-run 13.7ms / warm 11.7ms だった。
  // Node の V8 は workerd と同じエンジンだが、ハード・JIT の状態は違うので目安であり、
  // 本番の実測（`wrangler tail` の cpuTime、docs の手順）が正。落ちるのは極端に遅いときだけ。
  it("reports the validation cost for a body near MAX_BODY_BYTES", () => {
    const perField = Math.floor((MAX_BODY_BYTES - 300 * 1024) / 2 / 4) * 4;
    const body = JSON.stringify({
      ...validPayloadV4,
      log_excerpt_gz: "H4sI" + "A".repeat(perField - 4),
      app_log_excerpt_gz: "H4sI" + "B".repeat(perField - 4)
    });
    expect(body.length).toBeLessThanOrEqual(MAX_BODY_BYTES);
    expect(body.length).toBeGreaterThan(MAX_BODY_BYTES - 400 * 1024);

    const bytes = new TextEncoder().encode(body);
    // 共有ランナーでは同じ処理でも実行ごとに大きくぶれる（最適化の前後で 13.7ms → 18.8ms と
    // 逆転して見えた）ので、絶対値ではなく、同じ実行の中で「最適化前の処理」と「現在の処理」
    // を並べて比べる。最適化前 = 全文の文字種の正規表現 2 本 + 整形つき直列化。
    const fullScan = /^[A-Za-z0-9+/]*={0,2}$/;
    const legacyPipeline = (): void => {
      const text = new TextDecoder().decode(bytes);
      const payload = parseAndValidatePayload(text);
      fullScan.test(payload.log_excerpt_gz ?? "");
      fullScan.test(payload.app_log_excerpt_gz ?? "");
      JSON.stringify({ report_id: "x", received_at: "y", payload }, null, 2);
    };
    const currentPipeline = (): void => {
      const text = new TextDecoder().decode(bytes);
      const payload = parseAndValidatePayload(text);
      JSON.stringify({ report_id: "x", received_at: "y", payload });
    };
    const measure = (run: () => void): { first: number; warm: number } => {
      const times: number[] = [];
      for (let i = 0; i < 8; i += 1) {
        const start = performance.now();
        run();
        times.push(performance.now() - start);
      }
      return { first: times[0] ?? 0, warm: Math.min(...times.slice(1)) };
    };
    // 交互に測って、JIT・ランナーの状態の偏りを減らす。
    measure(legacyPipeline);
    measure(currentPipeline);
    const legacy = measure(legacyPipeline);
    const current = measure(currentPipeline);
    console.log(
      `[cpu] body=${(body.length / 1024).toFixed(0)}KiB ` +
        `legacy(first=${legacy.first.toFixed(1)} warm=${legacy.warm.toFixed(1)})ms ` +
        `current(first=${current.first.toFixed(1)} warm=${current.warm.toFixed(1)})ms ` +
        `ratio=${(current.warm / legacy.warm).toFixed(2)} ` +
        `(Workers Free の CPU 上限は 10ms/リクエスト)`
    );
    const first = current.first;
    // 現在の処理は、全文検証・整形つき直列化より速いはず（余裕を見て 0.9 倍未満）。
    expect(current.warm).toBeLessThan(legacy.warm * 0.9);
    expect(first).toBeLessThan(500);
  });
});

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

  // ADR-148: SCHEMA_VERSION は上げていないため、この変更より前のクライアント
  // （attach_ime_keymap/gji_keymap/msime_key_assignment を一切送らないペイロード）
  // が引き続き200で受理されることを固定する（retro_eval_stats と同型の回帰）。
  // legacy_msime_keymap（Phase 2）・keymap_learn（ADR196-T2）も同じ理由で
  // optionalとして読むため、ここに含める。
  it("accepts payloads without ime_keymap fields (pre-ADR-148 clients) and normalizes to false/null", () => {
    const {
      attach_ime_keymap: _attachImeKeymap,
      gji_keymap: _gjiKeymap,
      msime_key_assignment: _msimeKeyAssignment,
      legacy_msime_keymap: _legacyMsimeKeymap,
      keymap_learn: _keymapLearn,
      ...payload
    } = validPayload;

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual({
      ...payload,
      attach_ime_keymap: false,
      gji_keymap: null,
      msime_key_assignment: null,
      legacy_msime_keymap: null,
      keymap_learn: null
    });
  });

  // issue #165（hook_starved）用: SCHEMA_VERSION は上げていないため、この変更
  // より前のクライアント（attach_running_processes/running_processesを一切
  // 送らないペイロード）が引き続き200で受理されることを固定する。
  it("accepts payloads without running_processes fields (pre-issue-165 clients) and normalizes to false/null", () => {
    const {
      attach_running_processes: _attachRunningProcesses,
      running_processes: _runningProcesses,
      ...payload
    } = validPayload;

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual({
      ...payload,
      attach_running_processes: false,
      running_processes: null
    });
  });

  it("rejects a non-boolean attach_running_processes", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_running_processes: "yes"
      })),
      400,
      "attach_running_processes_invalid"
    );
  });

  it("rejects a non-array, non-null running_processes", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_running_processes: true,
        running_processes: 42
      })),
      400,
      "running_processes_invalid"
    );
  });

  it("rejects running_processes unless attach_running_processes is explicitly true", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_running_processes: false,
        running_processes: ["explorer.exe"]
      })),
      400,
      "running_processes_requires_attach_running_processes"
    );
  });

  it("accepts an explicitly attached running_processes array", () => {
    const payload = {
      ...validPayload,
      attach_running_processes: true,
      running_processes: ["explorer.exe", "powertoys.exe"]
    };

    expect(parseAndValidatePayload(JSON.stringify(payload))).toEqual(payload);
  });

  it("rejects a non-boolean attach_ime_keymap", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_ime_keymap: "yes"
      })),
      400,
      "attach_ime_keymap_invalid"
    );
  });

  it("rejects a non-object, non-null gji_keymap", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_ime_keymap: true,
        gji_keymap: 42
      })),
      400,
      "gji_keymap_invalid"
    );
  });

  it("rejects a non-object, non-null msime_key_assignment", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_ime_keymap: true,
        msime_key_assignment: 42
      })),
      400,
      "msime_key_assignment_invalid"
    );
  });

  it("rejects gji_keymap unless attach_ime_keymap is explicitly true", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_ime_keymap: false,
        gji_keymap: { session_keymap: 0 }
      })),
      400,
      "gji_keymap_requires_attach_ime_keymap"
    );
  });

  it("rejects msime_key_assignment unless attach_ime_keymap is explicitly true", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_ime_keymap: false,
        msime_key_assignment: { is_key_assignment_enabled: 1 }
      })),
      400,
      "msime_key_assignment_requires_attach_ime_keymap"
    );
  });

  it("rejects a non-object, non-null legacy_msime_keymap", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_ime_keymap: true,
        legacy_msime_keymap: 42
      })),
      400,
      "legacy_msime_keymap_invalid"
    );
  });

  it("rejects legacy_msime_keymap unless attach_ime_keymap is explicitly true", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_ime_keymap: false,
        legacy_msime_keymap: { muhenkan_ime_on_toggle: true }
      })),
      400,
      "legacy_msime_keymap_requires_attach_ime_keymap"
    );
  });

  it("rejects a non-object, non-null keymap_learn", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_ime_keymap: true,
        keymap_learn: 42
      })),
      400,
      "keymap_learn_invalid"
    );
  });

  it("rejects keymap_learn unless attach_ime_keymap is explicitly true", () => {
    expectHttpError(
      () => parseAndValidatePayload(JSON.stringify({
        ...validPayload,
        attach_ime_keymap: false,
        keymap_learn: { table_file: "loaded" }
      })),
      400,
      "keymap_learn_requires_attach_ime_keymap"
    );
  });

  it("accepts explicitly attached gji_keymap, msime_key_assignment, legacy_msime_keymap and keymap_learn objects", () => {
    const payload = {
      ...validPayload,
      attach_ime_keymap: true,
      gji_keymap: {
        config1_db_status: "Ok",
        session_keymap: 0,
        custom_keymap_table_is_effective: true,
        ime_on_keys: ["VK_F21"]
      },
      msime_key_assignment: {
        is_key_assignment_enabled: 1,
        key_assignment_muhenkan: 1
      },
      legacy_msime_keymap: {
        active_style: "Custom",
        muhenkan_ime_on_toggle: true,
        henkan_ime_on_toggle: false
      },
      keymap_learn: {
        table_file: "loaded",
        use_learned_keymap_table: true,
        in_use: true,
        cell_count: 42,
        judgement: "Accepted"
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

describe("release line classification", () => {
  it("classifies below the 1.90.0 threshold as v1", () => {
    expect(releaseLine(parseSemver("1.21.0")!)).toBe("v1");
    expect(releaseLine(parseSemver("1.89.999")!)).toBe("v1");
  });

  it("classifies 1.90.0 and above as v2, including a future 2.0.0", () => {
    expect(releaseLine(parseSemver("1.90.0")!)).toBe("v2");
    expect(releaseLine(parseSemver("1.95.3")!)).toBe("v2");
    expect(releaseLine(parseSemver("2.0.0")!)).toBe("v2");
  });
});

describe("line-aware latest release (?current_version=)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("picks the highest v1 tag and ignores v2 tags when the client is on the v1 line", async () => {
    mockGithubReleasesList([
      { tag_name: "v1.90.0" },
      { tag_name: "v1.22.0" },
      { tag_name: "v1.21.1" }
    ]);

    const response = await handleRequest(
      latestReleaseRequest({ currentVersion: "1.21.0" }),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toMatchObject({ latest_version: "1.22.0" });
  });

  it("picks the highest v2 tag and ignores v1 tags when the client is on the v2 line", async () => {
    mockGithubReleasesList([
      { tag_name: "v1.22.0" },
      { tag_name: "v1.95.0" },
      { tag_name: "v1.91.0" }
    ]);

    const response = await handleRequest(
      latestReleaseRequest({ currentVersion: "1.90.0" }),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toMatchObject({ latest_version: "1.95.0" });
  });

  it("ignores draft and prerelease entries when picking the per-line highest", async () => {
    mockGithubReleasesList([
      { tag_name: "v1.23.0", draft: true },
      { tag_name: "v1.22.5", prerelease: true },
      { tag_name: "v1.22.0" }
    ]);

    const response = await handleRequest(
      latestReleaseRequest({ currentVersion: "1.21.0" }),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    await expect(response.json()).resolves.toMatchObject({ latest_version: "1.22.0" });
  });

  it("falls back to the legacy global latest when current_version is malformed", async () => {
    const fetchMock = mockGithubLatestRelease("v1.19.0");

    const response = await handleRequest(
      latestReleaseRequest({ currentVersion: "not-a-version" }),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toMatchObject({ latest_version: "1.19.0" });
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "https://api.github.com/repos/cuzic/awase/releases/latest"
    );
  });

  it("caches v1 and v2 lines independently without cross-contamination", async () => {
    const kv = new MemoryKv();
    mockGithubReleasesList([{ tag_name: "v1.22.0" }, { tag_name: "v1.95.0" }]);

    await handleRequest(
      latestReleaseRequest({ currentVersion: "1.21.0" }),
      latestReleaseEnv(kv),
      fakeCtx([])
    );
    const v2Response = await handleRequest(
      latestReleaseRequest({ currentVersion: "1.90.0" }),
      latestReleaseEnv(kv),
      fakeCtx([])
    );

    expect(JSON.parse(kv.values.get("latest-release:line:v1") ?? "{}")).toMatchObject({
      latest_version: "1.22.0"
    });
    await expect(v2Response.json()).resolves.toMatchObject({ latest_version: "1.95.0" });
    expect(JSON.parse(kv.values.get("latest-release:line:v2") ?? "{}")).toMatchObject({
      latest_version: "1.95.0"
    });
  });

  it("returns 404 no_release_for_line when no release matches the requested line", async () => {
    mockGithubReleasesList([{ tag_name: "v1.22.0" }]);

    const response = await handleRequest(
      latestReleaseRequest({ currentVersion: "1.90.0" }),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    expect(response.status).toBe(404);
    await expect(response.json()).resolves.toEqual({ error: "no_release_for_line" });
  });

  // opusコードレビュー指摘: GitHub側の実障害と「該当ラインのリリースが単に無い」は
  // どちらもfetchLatestTagForLineの戻り値だけでは区別できなかった(常に404だった)。
  it("returns 503 upstream_unavailable (not 404) when GitHub is unavailable for a line-specific request", async () => {
    mockGithubResponse(new Response("rate limited", { status: 403 }));

    const response = await handleRequest(
      latestReleaseRequest({ currentVersion: "1.90.0" }),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    expect(response.status).toBe(503);
    await expect(response.json()).resolves.toEqual({ error: "upstream_unavailable" });
  });

  // opusコードレビュー指摘: per_page=100の1ページ目だけを見ていたため、v1/v2合計が
  // 100件を超えると古い方のラインの最新リリースがページ外に落ちて見えなくなっていた。
  it("paginates through GitHub releases when the requested line's latest release is past the first page", async () => {
    const firstPage = Array.from({ length: 100 }, (_, i) => ({ tag_name: `v1.95.${i}` }));
    const secondPage = [{ tag_name: "v1.22.0" }];
    const fetchMock = mockGithubReleasesPages([firstPage, secondPage]);

    const response = await handleRequest(
      latestReleaseRequest({ currentVersion: "1.21.0" }),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toMatchObject({ latest_version: "1.22.0" });
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(String(fetchMock.mock.calls[0]?.[0])).toContain("page=1");
    expect(String(fetchMock.mock.calls[1]?.[0])).toContain("page=2");
  });

  it("stops paginating once a short page is seen, without exceeding the page cap", async () => {
    const fetchMock = mockGithubReleasesPages([[{ tag_name: "v1.22.0" }]]);

    const response = await handleRequest(
      latestReleaseRequest({ currentVersion: "1.21.0" }),
      latestReleaseEnv(new MemoryKv()),
      fakeCtx([])
    );

    expect(response.status).toBe(200);
    await expect(response.json()).resolves.toMatchObject({ latest_version: "1.22.0" });
    expect(fetchMock).toHaveBeenCalledTimes(1);
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

function latestReleaseRequest(
  init?: RequestInit & { currentVersion?: string }
): Request {
  const { currentVersion, ...requestInit } = init ?? {};
  const url = new URL("https://report.awase.cc/v1/latest-release");
  if (currentVersion !== undefined) {
    url.searchParams.set("current_version", currentVersion);
  }
  return new Request(url, requestInit);
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

function mockGithubReleasesList(
  releases: Array<{ tag_name: string; draft?: boolean; prerelease?: boolean }>
) {
  return mockGithubResponse(Response.json(releases));
}

/** 呼び出しごとに異なるページ(`pages[0]`, `pages[1]`, ...)を返す。ページ数を超えた
 * 呼び出しには空配列を返す（`GITHUB_RELEASES_MAX_PAGES`到達時の安全側動作の検証用）。 */
function mockGithubReleasesPages(
  pages: Array<Array<{ tag_name: string; draft?: boolean; prerelease?: boolean }>>
) {
  let call = 0;
  const fetchMock = vi.fn(
    async (_input: RequestInfo | URL, _init?: RequestInit): Promise<Response> => {
      const page = pages[call] ?? [];
      call += 1;
      return Response.json(page);
    }
  );
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
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
  // `.clone()` so a mock can be read across multiple `fetch()` calls in one test
  // (a `Response` body can only be consumed once).
  const fetchMock = vi.fn(
    async (_input: RequestInfo | URL, _init?: RequestInit): Promise<Response> => response.clone()
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
