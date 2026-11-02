    MathFidelity math_fidelity = MathFidelity::HiFi4;
#ifdef TT_METAL_CI_MODE
        constexpr uint32_t M = 2048;  // CI values
        constexpr uint32_t N = 1024;  // CI values
        constexpr uint32_t K = 512;   // CI values
#else
        constexpr uint32_t M = 3584;  // tutorial values
        constexpr uint32_t N = 3072;  // tutorial values
        constexpr uint32_t K = 768;   // tutorial values
#endif
        TT_FATAL(pearson > 0.98, "PCC not high enough. Result PCC: {}, Expected PCC: 0.98", pearson);
        fmt::print("Test Passed\n");
