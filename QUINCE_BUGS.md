# QUINCE Bugs & Bottlenecks

Legend: ✅ fixed — ❌ not fixed

---

## P0 — ЖРУТ CPU / ПАМЯТЬ

### ✅ P0.1 Engine spin loop (busy wait)
**File:** `engine/src/loop.rs:214-216`
**Fix:** `yield_now()` → `tokio::time::sleep(Duration::from_millis(1)).await`. CPU простаивает между тиками.

### ✅ P0.2 LuaEvent channel — unbounded
**File:** `engine/src/loop.rs:66`
**Fix:** `unbounded()` → `bounded(1024)`. Старые тики молча дропаются, если Lua не успевает.

### ✅ P0.3 OrderManager — Cancelled/Failed never cleaned
**File:** `engine/src/orders.rs:123-131`
**Fix:** `cleanup_filled()` теперь также вычищает `Cancelled` / `Failed`.

### ✅ P0.4 MockExchange position zombie leak
**File:** `quince/src/mock.rs:185-196`
**Fix:** `state.positions.retain(|p| p.size > 0.0)` после каждой операции с позицией.

---

## P1 — ЛАТЕНТНОСТЬ / АЛЛОКАЦИИ

### ✅ P1.1 O(n) pending order scan на каждый fill
**File:** `engine/src/loop.rs:258-266`
**Fix:** `OrderManager.exchange_to_client: HashMap<String, String>`. `find_client_by_exchange_id()` — O(1).

### ✅ P1.2 deactivate_sl_tp клонирует весь Order
**File:** `engine/src/orders.rs:180-186`
**Fix:** `po.order.stop_loss = None; po.order.take_profit = None;` — прямая мутация.

### ✅ P1.3 active_sl_tp() аллоцирует Vec каждый тик
**File:** `engine/src/orders.rs:155-178`
**Fix:** быстрый `any()` чек перед аллокацией. Если нет Filled со SL/TP — возвращаем пустой Vec без аллокации.

### ✅ P1.4 k.to_string() на каждый индикатор каждый тик
**File:** `engine/src/loop.rs:248-253`
**Fix:** `HashMap<&'static str, f64>` — все имена индикаторов статические. Ноль String alloc в hot path.

### ✅ P1.5 Depth клонируется дважды
**File:** `engine/src/loop.rs:241-252`
**Fix:** Удалён `self.last_depth` (никем не читался) — остаётся 1 clone для Lua + move в ctx.depth.

### ✅ P1.6 Mutex contention на StrategyCtx
**File:** `engine/src/loop.rs:229-239` vs `strategy/src/runtime.rs:81-194`
**Fix:** P1.4 убрал `k.to_string()` из critical section. Engine держит лочку ~в 10x короче.

---

## P2 — МЕЛКИЕ

### ✅ P2.1 DepthLevel без Copy
**File:** `core/src/types.rs:27-31`
**Fix:** Добавлен `Copy`.

### ✅ P2.2 Mock current_price() всегда 100.0
**File:** `quince/src/mock.rs:237-239`
**Fix:** Читает `state.last_price`.

### ✅ P2.3 RingVec runtime modulo
**File:** `core/src/ring.rs:130,150,175`
**Fix:** Conditional subtract (`if idx >= cap { idx - cap } else { idx }`) вместо `%`. Избегает медленной инструкции деления, не меняет поведение.

### ✅ P2.4 RingBuffer<T,N> без тестов
**File:** `core/src/ring.rs:298`
**Fix:** Добавлены тесты: `ringbuffer_new_empty`, `ringbuffer_push_until_full`, `ringbuffer_evict_on_overflow`, `ringbuffer_get_logical_order`, `ringbuffer_last`, `ringbuffer_iter`, `ringbuffer_iter_after_eviction`, `ringbuffer_clear`, `ringbuffer_clear_reuse`, `ringbuffer_get_out_of_bounds`.

### ✅ P2.5 tokio::time::sleep(1ms) в WS task
**File:** `exchange/src/binance/ws.rs:166`
**Fix:** `sleep(1ms)` → `sleep(10ms)`. CPU нагрузка снижена в 10х.

### ✅ P2.6 text.as_bytes().to_vec() на каждое WS сообщение
**File:** `exchange/src/binance/types.rs:7`
**Fix:** `parse_ws_msg` принимает `String` и использует `text.into_bytes()` вместо копирования.

---

## P3 — СТИЛЬ

### ✅ P3.1 `_name` unused в default_buffer
**File:** `engine/src/indicators.rs:58`

### ✅ P3.2 Двойной Instant::now() в register()
**File:** `engine/src/orders.rs:58-59`

### ✅ P3.3 Двойной client_id.clone() в register()
**File:** `engine/src/orders.rs:53,55`

---

## История

| Дата | Фикс |
|------|------|
| 2026-06-03 | Удалён `latencies: Vec<f64>` (жрал 11GB за 20 мин) |
| 2026-06-03 | Удалён per-tick latency logger (спамил лог) |
| 2026-06-03 | P0.1-P0.4, P1.1-P1.6, P2.1-P2.2 — 12 багов пофикшено |
| 2026-06-03 | P2.3-P2.6, P3.1-P3.3 — 7 багов пофикшено |
