//! WASM host for the Behavior ABI (`behaviors` feature) — the real sandbox.
//!
//! Runs a world's untrusted WASM behavior module on a pure-Rust `wasmi` interpreter,
//! marshalling JSON events in and [`Action`]s out per behavior-abi-v0.1. The module
//! gets *no* ambient capabilities — only the host-fns below — and every call is
//! **fuel-bounded**, so a runaway or malicious module is killed and the world keeps
//! rendering (the super-stable tenet). Feature-gated because it pulls a WASM runtime.

use wasmi::{Caller, Engine, Extern, Linker, Memory, Module, Store, TypedFunc};

use crate::behavior::{parse_actions, Action, Behavior, InteractEvent};

/// CPU budget per handler call, in `wasmi` fuel units. A handler that exceeds it
/// traps and yields no actions — the world is never blocked by a bad module.
const FUEL_PER_CALL: u64 = 5_000_000;

/// Host-side state the sandbox can touch (only via the host-fns).
#[derive(Default)]
struct HostState {
    /// The world clock in ms (deterministic-friendly; not wall-clock).
    now_ms: i64,
    /// Actions a module emitted synchronously via `host_emit` during a call.
    emitted: Vec<Action>,
}

/// A loaded WASM behavior module instance.
pub struct WasmBehavior {
    store: Store<HostState>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    on_load: Option<TypedFunc<(), ()>>,
    on_interact: Option<TypedFunc<(i32, i32), i64>>,
    on_event: Option<TypedFunc<(i32, i32), ()>>,
    on_tick: Option<TypedFunc<i32, ()>>,
}

impl WasmBehavior {
    /// Instantiate a behavior module from its WASM bytes.
    pub fn load(wasm: &[u8]) -> Result<Self, String> {
        let mut config = wasmi::Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|e| e.to_string())?;
        let mut store = Store::new(&engine, HostState::default());
        let mut linker = <Linker<HostState>>::new(&engine);

        define_host_fns(&mut linker)?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| e.to_string())?
            .start(&mut store)
            .map_err(|e| e.to_string())?;

        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| "behavior module exports no `memory`".to_string())?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&store, "thread_alloc")
            .map_err(|_| "behavior module exports no `thread_alloc`".to_string())?;
        let on_load = instance
            .get_typed_func::<(), ()>(&store, "thread_on_load")
            .ok();
        let on_interact = instance
            .get_typed_func::<(i32, i32), i64>(&store, "thread_on_interact")
            .ok();
        let on_event = instance
            .get_typed_func::<(i32, i32), ()>(&store, "thread_on_event")
            .ok();
        let on_tick = instance
            .get_typed_func::<i32, ()>(&store, "thread_on_tick")
            .ok();

        let mut b = Self {
            store,
            memory,
            alloc,
            on_load,
            on_interact,
            on_event,
            on_tick,
        };
        if let Some(f) = b.on_load {
            b.refuel();
            let _ = f.call(&mut b.store, ());
        }
        Ok(b)
    }

    /// Set the world clock the module reads via `host_now_ms`.
    pub fn set_clock(&mut self, now_ms: i64) {
        self.store.data_mut().now_ms = now_ms;
    }

    fn refuel(&mut self) {
        // Reset the per-call budget (ignore if fuel isn't supported).
        let _ = self.store.add_fuel(FUEL_PER_CALL);
    }

    /// Write `bytes` into module memory via its allocator, returning the pointer.
    fn write_input(&mut self, bytes: &[u8]) -> Option<i32> {
        let ptr = self.alloc.call(&mut self.store, bytes.len() as i32).ok()?;
        self.memory
            .write(&mut self.store, ptr as usize, bytes)
            .ok()?;
        Some(ptr)
    }

    /// Read the `(ptr,len)` region a handler returned (packed `ptr<<32 | len`).
    fn read_output(&self, packed: i64) -> Vec<Action> {
        if packed == 0 {
            return Vec::new();
        }
        let p = packed as u64;
        let ptr = (p >> 32) as usize;
        let len = (p & 0xFFFF_FFFF) as usize;
        let data = self.memory.data(&self.store);
        match data.get(ptr..ptr.saturating_add(len)) {
            Some(bytes) => parse_actions(&String::from_utf8_lossy(bytes)),
            None => Vec::new(),
        }
    }
}

impl Behavior for WasmBehavior {
    fn on_load(&mut self) {} // already called during `load`

    fn on_interact(&mut self, event: &InteractEvent) -> Vec<Action> {
        let Some(handler) = self.on_interact else {
            return Vec::new();
        };
        let Ok(json) = serde_json::to_vec(event) else {
            return Vec::new();
        };
        // Refuel BEFORE write_input: it calls the module's `thread_alloc`, which
        // burns fuel too — on an empty tank it would trap before the handler ran.
        self.refuel();
        let Some(ptr) = self.write_input(&json) else {
            return Vec::new();
        };
        self.store.data_mut().emitted.clear();
        // A trap (fuel exhausted, panic, bad memory) yields no actions — never a crash.
        let packed = match handler.call(&mut self.store, (ptr, json.len() as i32)) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("behavior on_interact trapped: {e} (module killed for this call)");
                return std::mem::take(&mut self.store.data_mut().emitted);
            }
        };
        let mut actions = std::mem::take(&mut self.store.data_mut().emitted);
        actions.extend(self.read_output(packed));
        actions
    }

    fn on_tick(&mut self, dt_ms: i32) {
        if let Some(f) = self.on_tick {
            self.refuel();
            let _ = f.call(&mut self.store, dt_ms);
        }
    }

    /// Async host events (a settled `purchase_result`, `codex_ready`, …). Per
    /// the ABI, `thread_on_event` returns nothing — a module reacts by calling
    /// `host_emit`, so the emitted actions are what comes back here.
    fn on_event(&mut self, event: &serde_json::Value) -> Vec<Action> {
        let Some(handler) = self.on_event else {
            return Vec::new();
        };
        let Ok(json) = serde_json::to_vec(event) else {
            return Vec::new();
        };
        self.refuel();
        let Some(ptr) = self.write_input(&json) else {
            return Vec::new();
        };
        self.store.data_mut().emitted.clear();
        if let Err(e) = handler.call(&mut self.store, (ptr, json.len() as i32)) {
            tracing::warn!("behavior on_event trapped: {e} (module killed for this call)");
        }
        std::mem::take(&mut self.store.data_mut().emitted)
    }
}

/// Read a `(ptr,len)` string argument out of the caller's linear memory.
fn read_str(caller: &Caller<'_, HostState>, ptr: i32, len: i32) -> Option<String> {
    let Some(Extern::Memory(mem)) = caller.get_export("memory") else {
        return None;
    };
    let data = mem.data(caller);
    data.get(ptr as usize..(ptr as usize).saturating_add(len as usize))
        .map(|b| String::from_utf8_lossy(b).into_owned())
}

/// Bind the capability-gated host-fns a module may import. Everything with real side
/// effects goes through returned [`Action`]s instead — these are just log, clock,
/// and the `host_emit` sugar for firing an action mid-call.
fn define_host_fns(linker: &mut Linker<HostState>) -> Result<(), String> {
    linker
        .func_wrap(
            "env",
            "host_log",
            |caller: Caller<'_, HostState>, ptr: i32, len: i32| {
                if let Some(s) = read_str(&caller, ptr, len) {
                    tracing::debug!("behavior: {s}");
                }
            },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(
            "env",
            "host_now_ms",
            |caller: Caller<'_, HostState>| -> i64 { caller.data().now_ms },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(
            "env",
            "host_emit",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
                if let Some(s) = read_str(&caller, ptr, len) {
                    if let Ok(action) = serde_json::from_str::<Action>(&s) {
                        caller.data_mut().emitted.push(action);
                    }
                }
            },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior::Actor;

    /// A minimal conformant module in WAT: a bump allocator + an `on_interact`
    /// that returns one fixed `notify` action. This is the ABI's ground truth —
    /// if this stops passing, real modules break too.
    fn greeter_wasm() -> Vec<u8> {
        const REPLY: &str = r#"{"actions":[{"action":"notify","text":"hi from wasm"}]}"#;
        let wat = format!(
            r#"(module
              (memory (export "memory") 1)
              (global $heap (mut i32) (i32.const 4096))
              (data (i32.const 0) "{data}")
              (func (export "thread_alloc") (param i32) (result i32)
                (local $p i32)
                global.get $heap
                local.set $p
                global.get $heap
                local.get 0
                i32.add
                global.set $heap
                local.get $p)
              (func (export "thread_on_interact") (param i32 i32) (result i64)
                (; ptr 0, len {len} ;)
                i64.const {len}))"#,
            data = REPLY.replace('"', "\\\""),
            len = REPLY.len(),
        );
        wat::parse_str(&wat).expect("fixture WAT compiles")
    }

    fn interact_event() -> InteractEvent {
        InteractEvent {
            placement: "pedestal".into(),
            actor: Actor::default(),
            world: "test-world".into(),
            data: serde_json::Value::Null,
        }
    }

    #[test]
    fn a_wasm_module_speaks_the_abi_end_to_end() {
        let mut b = WasmBehavior::load(&greeter_wasm()).expect("module loads");
        let actions = b.on_interact(&interact_event());
        assert_eq!(
            actions,
            vec![Action::Notify {
                text: "hi from wasm".into(),
                level: None
            }],
            "the module's returned JSON came back as typed actions"
        );
        // A second dispatch works too (the instance persists between calls).
        assert_eq!(b.on_interact(&interact_event()).len(), 1);
    }

    #[test]
    fn a_module_without_handlers_yields_no_actions_not_an_error() {
        // Exports memory + alloc but no `thread_on_interact` — legal, inert.
        let wasm = wat::parse_str(
            r#"(module
              (memory (export "memory") 1)
              (func (export "thread_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap();
        let mut b = WasmBehavior::load(&wasm).expect("module loads");
        assert!(b.on_interact(&interact_event()).is_empty());
        b.on_tick(16); // also inert, also fine
    }

    #[test]
    fn async_events_reach_the_module_and_actions_come_back_via_emit() {
        // Per the ABI, `thread_on_event` returns nothing — the module reacts by
        // calling `host_emit`. This module emits one `notify` per event.
        const EMIT: &str = r#"{"action":"notify","text":"sold!"}"#;
        let wat = format!(
            r#"(module
              (import "env" "host_emit" (func $emit (param i32 i32)))
              (memory (export "memory") 1)
              (global $heap (mut i32) (i32.const 4096))
              (data (i32.const 0) "{data}")
              (func (export "thread_alloc") (param i32) (result i32)
                (local $p i32)
                global.get $heap local.set $p
                global.get $heap local.get 0 i32.add global.set $heap
                local.get $p)
              (func (export "thread_on_event") (param i32 i32)
                i32.const 0 i32.const {len} call $emit))"#,
            data = EMIT.replace('"', "\\\""),
            len = EMIT.len(),
        );
        let mut b = WasmBehavior::load(&wat::parse_str(&wat).unwrap()).expect("module loads");
        let event = serde_json::json!({"event": "purchase_result", "ok": true, "item": "21010001"});
        assert_eq!(
            b.on_event(&event),
            vec![Action::Notify {
                text: "sold!".into(),
                level: None
            }]
        );
        // A module without the export is inert, never an error.
        let mut plain = WasmBehavior::load(&greeter_wasm()).expect("module loads");
        assert!(plain.on_event(&event).is_empty());
    }

    #[test]
    fn a_runaway_module_is_fuel_killed_not_hung() {
        // `thread_on_interact` loops forever — the fuel budget must trap it.
        let wasm = wat::parse_str(
            r#"(module
              (memory (export "memory") 1)
              (func (export "thread_alloc") (param i32) (result i32) i32.const 0)
              (func (export "thread_on_interact") (param i32 i32) (result i64)
                (loop $spin br $spin)
                i64.const 0))"#,
        )
        .unwrap();
        let mut b = WasmBehavior::load(&wasm).expect("module loads");
        let actions = b.on_interact(&interact_event());
        assert!(
            actions.is_empty(),
            "the trap yields no actions, never a hang"
        );
    }
}
