    MathFidelity math_fidelity = MathFidelity::HiFi4;  // High fidelity math for accurate results
        constexpr uint32_t M = 640;  // Number of rows in matrix A (user-defined)
        constexpr uint32_t N = 640;  // Number of columns in matrix B (user-defined)
        constexpr uint32_t K = 640;  // Inner dimension for multiplication (user-defined)
        fmt::print("Metalium vs Golden -- PCC = {}\n", pearson);
        TT_FATAL(pearson > 0.97, "PCC not high enough. Result PCC: {}, Expected PCC: 0.97", pearson);
        fmt::print("Test Passed\n");
