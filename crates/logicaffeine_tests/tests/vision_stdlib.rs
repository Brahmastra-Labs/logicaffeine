//! LOGOS-Vision — fixed-point computer-vision kernels (`vision.lg`), the ecosystem foundation for
//! the FPGA fabric track. Fixed-point = plain integer arithmetic with one post-shift, which is how
//! CV is done on FPGAs (and maps directly onto the integer spatial fabric). `conv3x3Window` is the
//! 9-tap stencil primitive the supercompiler will specialize onto silicon; `conv3x3` streams it
//! over an image. Cross-tier locked (interp==VM) with exact integer values.

mod common;
use common::assert_all_tiers;

// ---- conv3x3Window: one output pixel from a 3×3 window and 3×3 kernel, with a Q-format shift ----

#[test]
fn conv3x3_window_identity_picks_center() {
    // Identity kernel (center = 1) with shift 0 returns the center pixel (=50).
    assert_all_tiers(
        "## Main\nShow conv3x3Window([10, 20, 30, 40, 50, 60, 70, 80, 90], [0, 0, 0, 0, 1, 0, 0, 0, 0], 0).\n",
        "50",
    );
}

#[test]
fn conv3x3_window_box_sum() {
    // All-ones kernel, shift 0: sum of the window 1..9 = 45.
    assert_all_tiers(
        "## Main\nShow conv3x3Window([1, 2, 3, 4, 5, 6, 7, 8, 9], [1, 1, 1, 1, 1, 1, 1, 1, 1], 0).\n",
        "45",
    );
}

#[test]
fn conv3x3_window_box_blur_with_shift() {
    // Same sum 45, shift 3 (÷8): 45 >> 3 = 5.
    assert_all_tiers(
        "## Main\nShow conv3x3Window([1, 2, 3, 4, 5, 6, 7, 8, 9], [1, 1, 1, 1, 1, 1, 1, 1, 1], 3).\n",
        "5",
    );
}

#[test]
fn conv3x3_window_sobel_signed_weights() {
    // Sobel-y [-1,-2,-1;0,0,0;1,2,1] on a horizontal ramp [1,1,1;2,2,2;3,3,3] = 8.
    assert_all_tiers(
        "## Main\nShow conv3x3Window([1, 1, 1, 2, 2, 2, 3, 3, 3], [-1, -2, -1, 0, 0, 0, 1, 2, 1], 0).\n",
        "8",
    );
}

#[test]
fn conv3x3_window_arithmetic_shift_is_signed() {
    // acc = 16·(-1) = -16; -16 >> 2 = -4 (arithmetic, sign-preserving).
    assert_all_tiers(
        "## Main\nShow conv3x3Window([16, 0, 0, 0, 0, 0, 0, 0, 0], [-1, 0, 0, 0, 0, 0, 0, 0, 0], 2).\n",
        "-4",
    );
}

// ---- conv3x3: valid 2D convolution over a w×h image → (w-2)×(h-2) output ----

#[test]
fn conv3x3_identity_over_image() {
    // 4×4 image, identity kernel, shift 0 → the inner 2×2 pixels (rows 2-3, cols 2-3).
    // img row-major 1..16; inner pixels are 6,7,10,11.
    assert_all_tiers(
        "## Main\nShow conv3x3(4, 4, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16], [0, 0, 0, 0, 1, 0, 0, 0, 0], 0).\n",
        "[6, 7, 10, 11]",
    );
}

#[test]
fn conv3x3_box_sum_over_image() {
    // 3×3 image (one output pixel), all-ones kernel = sum of all 9 = 45.
    assert_all_tiers(
        "## Main\nShow conv3x3(3, 3, [1, 2, 3, 4, 5, 6, 7, 8, 9], [1, 1, 1, 1, 1, 1, 1, 1, 1], 0).\n",
        "[45]",
    );
}
