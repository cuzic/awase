//! OS 非依存の観測結果型定義。
//!
//! ADR 028 により、フォーカス分類は Platform 層で完結する。
//! Engine は `FocusChanged` シグナル（データなし）を受けて
//! flush と lifecycle 整合のみ行う。
