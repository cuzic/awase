// UI test (spike): verify RESTRICTED_ACTUATION_CALL fires when `set_ime_open` is called
// from outside its designated call site (`set_ime_open_ordered`), regardless of whether
// the call uses dot-call or fully-qualified `::` syntax.

trait PlatformRuntime {
    fn set_ime_open(&mut self, open: bool) -> bool;

    fn apply_ime_open(&mut self, open: bool) -> bool {
        // Should trigger: called from the trait's own default body, which is not the
        // designated wrapper `set_ime_open_ordered`.
        self.set_ime_open(open)
    }
}

struct Fake;

impl PlatformRuntime for Fake {
    fn set_ime_open(&mut self, open: bool) -> bool {
        open
    }
}

impl Fake {
    // Should NOT trigger: this is the designated call site.
    fn set_ime_open_ordered(&mut self, open: bool) -> bool {
        PlatformRuntime::set_ime_open(self, open)
    }

    // Should trigger: qualified-path call from a non-designated function.
    fn rogue_caller(&mut self, open: bool) -> bool {
        PlatformRuntime::set_ime_open(self, open)
    }

    // Should trigger: dot-call from a non-designated function.
    fn another_rogue_caller(&mut self, open: bool) -> bool {
        self.set_ime_open(open)
    }

    // Should trigger (2026-09-09, opus code review S1): the restricted call is nested
    // inside a closure body. Before the S1 fix, `CallFinder` did not descend into
    // closure/async-block bodies (a separate HIR `Body` reached via `BodyId`), so this
    // call was silently invisible to the lint despite `closure_rogue_caller` itself not
    // being an allowed caller.
    fn closure_rogue_caller(&mut self, open: bool) -> bool {
        let mut go = || self.set_ime_open(open);
        go()
    }
}

fn main() {}
