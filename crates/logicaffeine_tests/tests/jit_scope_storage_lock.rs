//! Native list scopes free real storage while preserving caller and result buffers.
#![cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]

use logicaffeine_jit::{
    logos_rt_alloc_list_i64_fixed, logos_rt_alloc_scope_enter, logos_rt_alloc_scope_leave,
    logos_rt_region_alloc_reset, native_alloc_registry_len, native_alloc_scope_depth,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CountBytes;
thread_local! {
    static LIVE_BYTES: Cell<isize> = const { Cell::new(0) };
}
fn adjust(delta: isize) {
    let _ = LIVE_BYTES.try_with(|n| n.set(n.get() + delta));
}
unsafe impl GlobalAlloc for CountBytes {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            adjust(layout.size() as isize);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            adjust(layout.size() as isize);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        adjust(-(layout.size() as isize));
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = System.realloc(ptr, layout, size);
        if !next.is_null() {
            adjust(size as isize - layout.size() as isize);
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: CountBytes = CountBytes;

fn live_bytes() -> isize {
    LIVE_BYTES.with(Cell::get)
}

unsafe fn enter() {
    assert_eq!(
        logos_rt_alloc_scope_enter(std::ptr::null_mut(), 0, 0, 0, 0),
        1
    );
}
unsafe fn leave(returned: i64) {
    let mut frame = [returned];
    assert_eq!(
        logos_rt_alloc_scope_leave(frame.as_mut_ptr(), 0, 0, 1, 0),
        1
    );
    assert_eq!(frame[0], returned);
}
unsafe fn buffer(capacity: usize, value: i64) -> i64 {
    let mut frame = [0, 0, capacity as i64];
    logos_rt_alloc_list_i64_fixed(frame.as_mut_ptr(), 0, 1, 2);
    let items = &mut *(frame[0] as *mut Vec<i64>);
    assert!(items.capacity() >= capacity);
    items.push(value);
    frame[0]
}
unsafe fn first(handle: i64) -> i64 {
    (&*(handle as *const Vec<i64>))[0]
}
unsafe fn warm_bookkeeping() {
    assert_eq!(native_alloc_registry_len(), 0);
    assert_eq!(native_alloc_scope_depth(), 0);
    enter();
    enter();
    for _ in 0..4 {
        buffer(1, 0);
    }
    leave(0);
    leave(0);
}

#[test]
fn child_returns_free_scratch_storage_before_the_outer_boundary() {
    unsafe {
        warm_bookkeeping();
        let baseline = live_bytes();
        enter();
        let parent = buffer(64, 17);
        let with_parent = live_bytes();
        assert!(with_parent >= baseline + 64 * 8);
        for _ in 0..128 {
            enter();
            buffer(4096, 23);
            buffer(8192, 29);
            assert!(live_bytes() >= with_parent + (4096 + 8192) * 8);
            leave(0);
            assert_eq!(native_alloc_registry_len(), 1);
            assert_eq!(native_alloc_scope_depth(), 1);
            assert_eq!(
                live_bytes(),
                with_parent,
                "scratch buffers survived their scope"
            );
            assert_eq!(first(parent), 17);
        }
        leave(0);
        assert_eq!(live_bytes(), baseline);
        assert_eq!(native_alloc_registry_len(), 0);
        assert_eq!(native_alloc_scope_depth(), 0);
    }
}

#[test]
fn a_child_preserves_its_owned_result_and_a_borrowed_return() {
    unsafe {
        warm_bookkeeping();
        let mut borrowed = vec![91_i64; 32];
        let borrowed_handle = &mut borrowed as *mut Vec<i64> as i64;
        let baseline = live_bytes();
        enter();
        let parent = buffer(64, 17);
        let with_parent = live_bytes();

        enter();
        buffer(4096, 23);
        leave(borrowed_handle);
        assert_eq!(live_bytes(), with_parent);
        assert_eq!(borrowed[0], 91);
        assert_eq!(first(parent), 17);

        enter();
        let returned = buffer(4096, 29);
        let with_result = live_bytes();
        buffer(8192, 31);
        leave(returned);
        assert_eq!(
            live_bytes(),
            with_result,
            "only the result buffer should survive"
        );
        assert_eq!(native_alloc_registry_len(), 2);
        assert_eq!(first(returned), 29);
        assert_eq!(first(parent), 17);

        leave(0);
        assert_eq!(live_bytes(), baseline);
        assert_eq!(borrowed[0], 91);
        assert_eq!(native_alloc_registry_len(), 0);
        assert_eq!(native_alloc_scope_depth(), 0);
    }
}

#[test]
fn a_region_reset_reclaims_storage_and_clears_nested_scopes() {
    unsafe {
        warm_bookkeeping();
        let baseline = live_bytes();
        enter();
        buffer(4096, 17);
        enter();
        buffer(8192, 23);
        assert!(live_bytes() >= baseline + (4096 + 8192) * 8);
        logos_rt_region_alloc_reset(std::ptr::null_mut(), 0, 0, 0, 0);
        assert_eq!(live_bytes(), baseline);
        assert_eq!(native_alloc_registry_len(), 0);
        assert_eq!(native_alloc_scope_depth(), 0);
    }
}

#[test]
fn allocation_helpers_obey_host_success_and_result_contract() {
    unsafe {
        let mut frame = [0_i64, 19, -1];
        let entered = logos_rt_alloc_scope_enter(frame.as_mut_ptr(), 2, 0, 0, 0);
        let left = logos_rt_alloc_scope_leave(frame.as_mut_ptr(), 2, 1, 1, 1);
        let reset = logos_rt_region_alloc_reset(frame.as_mut_ptr(), 2, 0, 0, 0);
        assert_eq!((entered, left, reset), (1, 1, 1));
        assert_eq!(frame[2], 19);
    }
}

#[test]
fn child_preserves_all_exported_mutable_lists_and_deduplicates_aliases() {
    unsafe {
        warm_bookkeeping();
        let baseline = live_bytes();
        enter();
        let parent = buffer(64, 17);
        enter();
        let first_output = buffer(4096, 29);
        let second_output = buffer(8192, 31);
        let with_outputs = live_bytes();
        buffer(16384, 37);
        let mut frame = [-1, 23, first_output, second_output, first_output];
        let status = logos_rt_alloc_scope_leave(frame.as_mut_ptr(), 0, 1, 4, 1);
        // Check census before dereferencing exports, so a broken keeper fails safely.
        assert_eq!(native_alloc_registry_len(), 3);
        assert_eq!(live_bytes(), with_outputs);
        assert_eq!(status, 1);
        assert_eq!(frame[0], 23);
        assert_eq!(first(parent), 17);
        assert_eq!(first(first_output), 29);
        assert_eq!(first(second_output), 31);
        logos_rt_region_alloc_reset(std::ptr::null_mut(), 0, 0, 0, 0);
        assert_eq!(live_bytes(), baseline);
        assert_eq!(native_alloc_scope_depth(), 0);
        assert_eq!(native_alloc_registry_len(), 0);
    }
}

#[test]
fn list_adoption_obeys_the_declared_host_window() {
    unsafe {
        warm_bookkeeping();
        let mut destination = vec![5_i64, 6];
        enter();
        let returned = buffer(128, 41);
        let handle = &mut destination as *mut Vec<i64> as i64;
        let mut frame = [-1, returned, handle, 12345, 67890];
        let status = logicaffeine_jit::logos_rt_adopt_list_i64(frame.as_mut_ptr(), 0, 1, 2, 0);
        assert_eq!(native_alloc_registry_len(), 0);
        assert_eq!(destination, vec![41]);
        assert_eq!(status, 1);
        assert_eq!(frame[0], handle);
        assert_eq!(&frame[3..], &[12345, 67890], "helper wrote outside dst");
        leave(0);
    }
}

#[test]
fn list_adoption_copies_borrowed_returns_and_preserves_identity() {
    unsafe {
        warm_bookkeeping();
        let mut borrowed = vec![71_i64, 73];
        let mut destination = vec![5_i64];
        let source = &mut borrowed as *mut Vec<i64> as i64;
        let handle = &mut destination as *mut Vec<i64> as i64;
        let mut frame = [-1, source, handle, 12345, 67890];
        let copied = logicaffeine_jit::logos_rt_adopt_list_i64(frame.as_mut_ptr(), 0, 1, 2, 0);
        assert_eq!(destination, borrowed);
        destination[0] = 79;
        assert_eq!(borrowed[0], 71);
        frame[1] = handle;
        let same = logicaffeine_jit::logos_rt_adopt_list_i64(frame.as_mut_ptr(), 0, 1, 2, 0);
        assert_eq!((copied, same), (1, 1));
        assert_eq!(frame[0], handle);
        assert_eq!(&frame[3..], &[12345, 67890]);
        assert_eq!(destination, vec![79, 73]);
        assert_eq!(native_alloc_registry_len(), 0);
    }
}

#[test]
fn scalar_return_bits_do_not_keep_a_buffer_with_the_same_address() {
    unsafe {
        warm_bookkeeping();
        let baseline = live_bytes();
        enter();
        let scalar_bits = buffer(4096, 17);
        let mut frame = [scalar_bits, -1];
        assert_eq!(
            logos_rt_alloc_scope_leave(frame.as_mut_ptr(), 1, 0, 1, 1),
            1
        );
        assert_eq!(frame[1], scalar_bits);
        assert_eq!(
            native_alloc_registry_len(),
            0,
            "a scalar cannot export ownership"
        );
        assert_eq!(native_alloc_scope_depth(), 0);
        assert_eq!(live_bytes(), baseline);
    }
}
