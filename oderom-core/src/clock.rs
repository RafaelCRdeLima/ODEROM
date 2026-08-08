//! A monotonic clock that also exists in the browser.
//!
//! `std::time::Instant::now()` **panics** on `wasm32-unknown-unknown`:
//! that target has no clock of its own, and the standard library's
//! answer is an `unreachable`, not an error a caller could handle. Any
//! `Instant::now()` on a code path the web build reaches is therefore a
//! crash waiting for the first student to run that path -- which is
//! exactly how this module came to exist (`Session::evaluate_definitions`
//! aborted the wasm module on the very first execute).
//!
//! [`Instant`] here is a drop-in replacement for the standard one, for
//! the narrow use this codebase makes of it (start a stopwatch, ask how
//! long it has been). Off wasm it *is* `std::time::Instant`, with the
//! same monotonicity guarantee and no measurable overhead.
//!
//! On wasm it reads whatever function the host registered with
//! [`set_time_source`], and reports zero if none was -- because the one
//! thing this must never do is bring back the panic it exists to
//! remove. `oderom-wasm` registers the browser's `Date.now` at startup;
//! `oderom-core` itself stays free of any JS dependency, which is why
//! the source is injected rather than imported.
//!
//! Nothing in `oderom-session`/`oderom-notebook` *decides* anything from
//! these numbers -- they are reported to the user (`elapsed_ms`) and
//! nothing more. The CLI's `--timeout`, which does decide, is enforced
//! in `oderom-cli` and never reached by the browser build.

use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug)]
pub struct Instant(std::time::Instant);

#[cfg(not(target_arch = "wasm32"))]
impl Instant {
    pub fn now() -> Self {
        Instant(std::time::Instant::now())
    }

    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

/// Sem efeito fora do wasm: aqui já existe um relógio, e ignorar a
/// fonte oferecida é o comportamento certo.
///
/// A função existe nos dois alvos, em vez de só no wasm, para que quem
/// chama não precise de um `#[cfg]` -- `oderom-wasm` é um crate
/// `cdylib` mas também compila para o host (é assim que seus testes
/// rodam), e um setup que só existisse em um dos dois transformaria
/// cada chamador num segundo lugar para lembrar do alvo.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_time_source(_source: fn() -> f64) {}

/// Milliseconds since some fixed origin -- only differences are ever
/// used, so the origin itself is irrelevant.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug)]
pub struct Instant(f64);

#[cfg(target_arch = "wasm32")]
static TIME_SOURCE: std::sync::OnceLock<fn() -> f64> = std::sync::OnceLock::new();

/// Registers where the milliseconds come from. Call once, at startup,
/// before anything measures anything; later calls are ignored (a clock
/// that changed origin mid-run would make an in-flight measurement
/// report nonsense, and silently keeping the first is the honest way to
/// make that impossible).
#[cfg(target_arch = "wasm32")]
pub fn set_time_source(source: fn() -> f64) {
    let _ = TIME_SOURCE.set(source);
}

#[cfg(target_arch = "wasm32")]
impl Instant {
    pub fn now() -> Self {
        Instant(TIME_SOURCE.get().map_or(0.0, |source| source()))
    }

    pub fn elapsed(&self) -> Duration {
        // `Date.now` is wall-clock, not monotonic: an NTP correction or
        // the user changing the system clock mid-computation can make
        // this go backwards. `max(0.0)` keeps that from becoming a
        // panic inside `from_secs_f64`, at the cost of a measurement
        // that reads as instantaneous -- the right trade for a number
        // whose only job is to be displayed.
        let ms = (Self::now().0 - self.0).max(0.0);
        Duration::from_secs_f64(ms / 1000.0)
    }
}
