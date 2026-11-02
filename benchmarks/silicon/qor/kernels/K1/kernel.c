/* K1 decide — the HFT decision of crates/logicaffeine_compile/src/codegen_sva/decision_workload.rs (DECIDE_FN),
   statement for statement. Sums and differences are formed in uint64_t, so they wrap as the 64-bit fabric does
   and carry no signed-overflow assumption into the HLS front end; comparisons are signed, as in the program. */
#include <stdint.h>

int64_t decide(int64_t micro100, int64_t fair100, int64_t edge100, int64_t base_size,
               int64_t max_order, int64_t inventory, int64_t pos_cap, int64_t book_ok) {
  int64_t order_side = 0;
  int64_t order_size = 0;
  if (book_ok == 1) {
    if (micro100 > (int64_t)((uint64_t)fair100 + (uint64_t)edge100)) {
      order_side = 2;
      order_size = base_size;
    } else if (micro100 < (int64_t)((uint64_t)fair100 - (uint64_t)edge100)) {
      order_side = 1;
      order_size = base_size;
    }
  }
  if (order_size > max_order) order_size = max_order;
  int64_t signed_order = 0;
  if (order_side == 1) signed_order = order_size;
  if (order_side == 2) signed_order = (int64_t)(0 - (uint64_t)order_size);
  int64_t projected = (int64_t)((uint64_t)inventory + (uint64_t)signed_order);
  int64_t risk_ok = 1;
  if (projected > pos_cap) risk_ok = 0;
  if (projected < (int64_t)(0 - (uint64_t)pos_cap)) risk_ok = 0;
  if (risk_ok == 0) signed_order = 0;
  return signed_order;
}
