// fixture: a legacy emitted host (bare LOGOS_MM_PASS) whose knobs live only in source
int main() {
    CreateKernel(program, "kernels/compute/logos_mm.cpp", all_cores,
        ComputeConfig{.math_fidelity = MathFidelity::HiFi4, .fp32_dest_acc_en = true, .compile_args = {}});
    if (pcc > 0.99) fmt::print("LOGOS_MM_PASS\n");
    return 0;
}
