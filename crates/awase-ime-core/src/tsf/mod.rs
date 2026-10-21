//! TSF の純粋なデータ型（殻の `awase-windows::tsf` が再公開する）。
//!
//! `tsf/` は複数の crate にまたがる（ADR-274 段階 2）。`converter_target` は土台の `awase-windows-base` にあり、
//! ここで再公開する。この crate が持つのは `literal_facts` だけ。

pub use awase_windows_base::tsf::*;

pub mod literal_facts;
