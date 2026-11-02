// secded_enc — SECDED(72,64) encoder (Hamming SEC + overall parity).
// GENERATED from the same incidence as codegen_sva::scratchpad_rtl::encode.
module secded_enc(input [63:0] din, output [71:0] cw);
  wire p3 = din[0];
  wire p5 = din[1];
  wire p6 = din[2];
  wire p7 = din[3];
  wire p9 = din[4];
  wire p10 = din[5];
  wire p11 = din[6];
  wire p12 = din[7];
  wire p13 = din[8];
  wire p14 = din[9];
  wire p15 = din[10];
  wire p17 = din[11];
  wire p18 = din[12];
  wire p19 = din[13];
  wire p20 = din[14];
  wire p21 = din[15];
  wire p22 = din[16];
  wire p23 = din[17];
  wire p24 = din[18];
  wire p25 = din[19];
  wire p26 = din[20];
  wire p27 = din[21];
  wire p28 = din[22];
  wire p29 = din[23];
  wire p30 = din[24];
  wire p31 = din[25];
  wire p33 = din[26];
  wire p34 = din[27];
  wire p35 = din[28];
  wire p36 = din[29];
  wire p37 = din[30];
  wire p38 = din[31];
  wire p39 = din[32];
  wire p40 = din[33];
  wire p41 = din[34];
  wire p42 = din[35];
  wire p43 = din[36];
  wire p44 = din[37];
  wire p45 = din[38];
  wire p46 = din[39];
  wire p47 = din[40];
  wire p48 = din[41];
  wire p49 = din[42];
  wire p50 = din[43];
  wire p51 = din[44];
  wire p52 = din[45];
  wire p53 = din[46];
  wire p54 = din[47];
  wire p55 = din[48];
  wire p56 = din[49];
  wire p57 = din[50];
  wire p58 = din[51];
  wire p59 = din[52];
  wire p60 = din[53];
  wire p61 = din[54];
  wire p62 = din[55];
  wire p63 = din[56];
  wire p65 = din[57];
  wire p66 = din[58];
  wire p67 = din[59];
  wire p68 = din[60];
  wire p69 = din[61];
  wire p70 = din[62];
  wire p71 = din[63];
  wire p1 = ^{din[0], din[1], din[3], din[4], din[6], din[8], din[10], din[11], din[13], din[15], din[17], din[19], din[21], din[23], din[25], din[26], din[28], din[30], din[32], din[34], din[36], din[38], din[40], din[42], din[44], din[46], din[48], din[50], din[52], din[54], din[56], din[57], din[59], din[61], din[63]};
  wire p2 = ^{din[0], din[2], din[3], din[5], din[6], din[9], din[10], din[12], din[13], din[16], din[17], din[20], din[21], din[24], din[25], din[27], din[28], din[31], din[32], din[35], din[36], din[39], din[40], din[43], din[44], din[47], din[48], din[51], din[52], din[55], din[56], din[58], din[59], din[62], din[63]};
  wire p4 = ^{din[1], din[2], din[3], din[7], din[8], din[9], din[10], din[14], din[15], din[16], din[17], din[22], din[23], din[24], din[25], din[29], din[30], din[31], din[32], din[37], din[38], din[39], din[40], din[45], din[46], din[47], din[48], din[53], din[54], din[55], din[56], din[60], din[61], din[62], din[63]};
  wire p8 = ^{din[4], din[5], din[6], din[7], din[8], din[9], din[10], din[18], din[19], din[20], din[21], din[22], din[23], din[24], din[25], din[33], din[34], din[35], din[36], din[37], din[38], din[39], din[40], din[49], din[50], din[51], din[52], din[53], din[54], din[55], din[56]};
  wire p16 = ^{din[11], din[12], din[13], din[14], din[15], din[16], din[17], din[18], din[19], din[20], din[21], din[22], din[23], din[24], din[25], din[41], din[42], din[43], din[44], din[45], din[46], din[47], din[48], din[49], din[50], din[51], din[52], din[53], din[54], din[55], din[56]};
  wire p32 = ^{din[26], din[27], din[28], din[29], din[30], din[31], din[32], din[33], din[34], din[35], din[36], din[37], din[38], din[39], din[40], din[41], din[42], din[43], din[44], din[45], din[46], din[47], din[48], din[49], din[50], din[51], din[52], din[53], din[54], din[55], din[56]};
  wire p64 = ^{din[57], din[58], din[59], din[60], din[61], din[62], din[63]};
  wire p0 = ^{din[0], din[1], din[2], din[4], din[5], din[7], din[10], din[11], din[12], din[14], din[17], din[18], din[21], din[23], din[24], din[26], din[27], din[29], din[32], din[33], din[36], din[38], din[39], din[41], din[44], din[46], din[47], din[50], din[51], din[53], din[56], din[57], din[58], din[60], din[63]};
  assign cw = {p71, p70, p69, p68, p67, p66, p65, p64, p63, p62, p61, p60, p59, p58, p57, p56, p55, p54, p53, p52, p51, p50, p49, p48, p47, p46, p45, p44, p43, p42, p41, p40, p39, p38, p37, p36, p35, p34, p33, p32, p31, p30, p29, p28, p27, p26, p25, p24, p23, p22, p21, p20, p19, p18, p17, p16, p15, p14, p13, p12, p11, p10, p9, p8, p7, p6, p5, p4, p3, p2, p1, p0};
endmodule

// secded_dec — SECDED(72,64) decoder: corrects any single-bit error,
// detects any double-bit error (double_err), recovers the 64 data bits.
module secded_dec(input [71:0] cw_in, output [63:0] dout,
                  output corrected, output double_err);
  wire q0 = cw_in[0];
  wire q1 = cw_in[1];
  wire q2 = cw_in[2];
  wire q3 = cw_in[3];
  wire q4 = cw_in[4];
  wire q5 = cw_in[5];
  wire q6 = cw_in[6];
  wire q7 = cw_in[7];
  wire q8 = cw_in[8];
  wire q9 = cw_in[9];
  wire q10 = cw_in[10];
  wire q11 = cw_in[11];
  wire q12 = cw_in[12];
  wire q13 = cw_in[13];
  wire q14 = cw_in[14];
  wire q15 = cw_in[15];
  wire q16 = cw_in[16];
  wire q17 = cw_in[17];
  wire q18 = cw_in[18];
  wire q19 = cw_in[19];
  wire q20 = cw_in[20];
  wire q21 = cw_in[21];
  wire q22 = cw_in[22];
  wire q23 = cw_in[23];
  wire q24 = cw_in[24];
  wire q25 = cw_in[25];
  wire q26 = cw_in[26];
  wire q27 = cw_in[27];
  wire q28 = cw_in[28];
  wire q29 = cw_in[29];
  wire q30 = cw_in[30];
  wire q31 = cw_in[31];
  wire q32 = cw_in[32];
  wire q33 = cw_in[33];
  wire q34 = cw_in[34];
  wire q35 = cw_in[35];
  wire q36 = cw_in[36];
  wire q37 = cw_in[37];
  wire q38 = cw_in[38];
  wire q39 = cw_in[39];
  wire q40 = cw_in[40];
  wire q41 = cw_in[41];
  wire q42 = cw_in[42];
  wire q43 = cw_in[43];
  wire q44 = cw_in[44];
  wire q45 = cw_in[45];
  wire q46 = cw_in[46];
  wire q47 = cw_in[47];
  wire q48 = cw_in[48];
  wire q49 = cw_in[49];
  wire q50 = cw_in[50];
  wire q51 = cw_in[51];
  wire q52 = cw_in[52];
  wire q53 = cw_in[53];
  wire q54 = cw_in[54];
  wire q55 = cw_in[55];
  wire q56 = cw_in[56];
  wire q57 = cw_in[57];
  wire q58 = cw_in[58];
  wire q59 = cw_in[59];
  wire q60 = cw_in[60];
  wire q61 = cw_in[61];
  wire q62 = cw_in[62];
  wire q63 = cw_in[63];
  wire q64 = cw_in[64];
  wire q65 = cw_in[65];
  wire q66 = cw_in[66];
  wire q67 = cw_in[67];
  wire q68 = cw_in[68];
  wire q69 = cw_in[69];
  wire q70 = cw_in[70];
  wire q71 = cw_in[71];
  wire s0 = ^{q1, q3, q5, q7, q9, q11, q13, q15, q17, q19, q21, q23, q25, q27, q29, q31, q33, q35, q37, q39, q41, q43, q45, q47, q49, q51, q53, q55, q57, q59, q61, q63, q65, q67, q69, q71};
  wire s1 = ^{q2, q3, q6, q7, q10, q11, q14, q15, q18, q19, q22, q23, q26, q27, q30, q31, q34, q35, q38, q39, q42, q43, q46, q47, q50, q51, q54, q55, q58, q59, q62, q63, q66, q67, q70, q71};
  wire s2 = ^{q4, q5, q6, q7, q12, q13, q14, q15, q20, q21, q22, q23, q28, q29, q30, q31, q36, q37, q38, q39, q44, q45, q46, q47, q52, q53, q54, q55, q60, q61, q62, q63, q68, q69, q70, q71};
  wire s3 = ^{q8, q9, q10, q11, q12, q13, q14, q15, q24, q25, q26, q27, q28, q29, q30, q31, q40, q41, q42, q43, q44, q45, q46, q47, q56, q57, q58, q59, q60, q61, q62, q63};
  wire s4 = ^{q16, q17, q18, q19, q20, q21, q22, q23, q24, q25, q26, q27, q28, q29, q30, q31, q48, q49, q50, q51, q52, q53, q54, q55, q56, q57, q58, q59, q60, q61, q62, q63};
  wire s5 = ^{q32, q33, q34, q35, q36, q37, q38, q39, q40, q41, q42, q43, q44, q45, q46, q47, q48, q49, q50, q51, q52, q53, q54, q55, q56, q57, q58, q59, q60, q61, q62, q63};
  wire s6 = ^{q64, q65, q66, q67, q68, q69, q70, q71};
  wire [6:0] syndrome = {s6, s5, s4, s3, s2, s1, s0};
  wire odd_err = ^cw_in;
  wire syn_nz = |syndrome;
  wire beyond = (s6 & (s5 | (s4 | s3)));
  assign corrected = odd_err & ~beyond;
  assign double_err = syn_nz & (~odd_err | beyond);
  wire f0 = q3 ^ (odd_err & (syndrome == 3)); // pos 3
  wire f1 = q5 ^ (odd_err & (syndrome == 5)); // pos 5
  wire f2 = q6 ^ (odd_err & (syndrome == 6)); // pos 6
  wire f3 = q7 ^ (odd_err & (syndrome == 7)); // pos 7
  wire f4 = q9 ^ (odd_err & (syndrome == 9)); // pos 9
  wire f5 = q10 ^ (odd_err & (syndrome == 10)); // pos 10
  wire f6 = q11 ^ (odd_err & (syndrome == 11)); // pos 11
  wire f7 = q12 ^ (odd_err & (syndrome == 12)); // pos 12
  wire f8 = q13 ^ (odd_err & (syndrome == 13)); // pos 13
  wire f9 = q14 ^ (odd_err & (syndrome == 14)); // pos 14
  wire f10 = q15 ^ (odd_err & (syndrome == 15)); // pos 15
  wire f11 = q17 ^ (odd_err & (syndrome == 17)); // pos 17
  wire f12 = q18 ^ (odd_err & (syndrome == 18)); // pos 18
  wire f13 = q19 ^ (odd_err & (syndrome == 19)); // pos 19
  wire f14 = q20 ^ (odd_err & (syndrome == 20)); // pos 20
  wire f15 = q21 ^ (odd_err & (syndrome == 21)); // pos 21
  wire f16 = q22 ^ (odd_err & (syndrome == 22)); // pos 22
  wire f17 = q23 ^ (odd_err & (syndrome == 23)); // pos 23
  wire f18 = q24 ^ (odd_err & (syndrome == 24)); // pos 24
  wire f19 = q25 ^ (odd_err & (syndrome == 25)); // pos 25
  wire f20 = q26 ^ (odd_err & (syndrome == 26)); // pos 26
  wire f21 = q27 ^ (odd_err & (syndrome == 27)); // pos 27
  wire f22 = q28 ^ (odd_err & (syndrome == 28)); // pos 28
  wire f23 = q29 ^ (odd_err & (syndrome == 29)); // pos 29
  wire f24 = q30 ^ (odd_err & (syndrome == 30)); // pos 30
  wire f25 = q31 ^ (odd_err & (syndrome == 31)); // pos 31
  wire f26 = q33 ^ (odd_err & (syndrome == 33)); // pos 33
  wire f27 = q34 ^ (odd_err & (syndrome == 34)); // pos 34
  wire f28 = q35 ^ (odd_err & (syndrome == 35)); // pos 35
  wire f29 = q36 ^ (odd_err & (syndrome == 36)); // pos 36
  wire f30 = q37 ^ (odd_err & (syndrome == 37)); // pos 37
  wire f31 = q38 ^ (odd_err & (syndrome == 38)); // pos 38
  wire f32 = q39 ^ (odd_err & (syndrome == 39)); // pos 39
  wire f33 = q40 ^ (odd_err & (syndrome == 40)); // pos 40
  wire f34 = q41 ^ (odd_err & (syndrome == 41)); // pos 41
  wire f35 = q42 ^ (odd_err & (syndrome == 42)); // pos 42
  wire f36 = q43 ^ (odd_err & (syndrome == 43)); // pos 43
  wire f37 = q44 ^ (odd_err & (syndrome == 44)); // pos 44
  wire f38 = q45 ^ (odd_err & (syndrome == 45)); // pos 45
  wire f39 = q46 ^ (odd_err & (syndrome == 46)); // pos 46
  wire f40 = q47 ^ (odd_err & (syndrome == 47)); // pos 47
  wire f41 = q48 ^ (odd_err & (syndrome == 48)); // pos 48
  wire f42 = q49 ^ (odd_err & (syndrome == 49)); // pos 49
  wire f43 = q50 ^ (odd_err & (syndrome == 50)); // pos 50
  wire f44 = q51 ^ (odd_err & (syndrome == 51)); // pos 51
  wire f45 = q52 ^ (odd_err & (syndrome == 52)); // pos 52
  wire f46 = q53 ^ (odd_err & (syndrome == 53)); // pos 53
  wire f47 = q54 ^ (odd_err & (syndrome == 54)); // pos 54
  wire f48 = q55 ^ (odd_err & (syndrome == 55)); // pos 55
  wire f49 = q56 ^ (odd_err & (syndrome == 56)); // pos 56
  wire f50 = q57 ^ (odd_err & (syndrome == 57)); // pos 57
  wire f51 = q58 ^ (odd_err & (syndrome == 58)); // pos 58
  wire f52 = q59 ^ (odd_err & (syndrome == 59)); // pos 59
  wire f53 = q60 ^ (odd_err & (syndrome == 60)); // pos 60
  wire f54 = q61 ^ (odd_err & (syndrome == 61)); // pos 61
  wire f55 = q62 ^ (odd_err & (syndrome == 62)); // pos 62
  wire f56 = q63 ^ (odd_err & (syndrome == 63)); // pos 63
  wire f57 = q65 ^ (odd_err & (syndrome == 65)); // pos 65
  wire f58 = q66 ^ (odd_err & (syndrome == 66)); // pos 66
  wire f59 = q67 ^ (odd_err & (syndrome == 67)); // pos 67
  wire f60 = q68 ^ (odd_err & (syndrome == 68)); // pos 68
  wire f61 = q69 ^ (odd_err & (syndrome == 69)); // pos 69
  wire f62 = q70 ^ (odd_err & (syndrome == 70)); // pos 70
  wire f63 = q71 ^ (odd_err & (syndrome == 71)); // pos 71
  assign dout = {f63, f62, f61, f60, f59, f58, f57, f56, f55, f54, f53, f52, f51, f50, f49, f48, f47, f46, f45, f44, f43, f42, f41, f40, f39, f38, f37, f36, f35, f34, f33, f32, f31, f30, f29, f28, f27, f26, f25, f24, f23, f22, f21, f20, f19, f18, f17, f16, f15, f14, f13, f12, f11, f10, f9, f8, f7, f6, f5, f4, f3, f2, f1, f0};
endmodule

// double_buffer — ping-pong scratchpad bank select. Compute reads read_bank while
// the DMA fills fill_bank; a swap flips both. read_bank != fill_bank by construction.
module double_buffer(input clk, input rst, input swap,
                     output reg read_bank, output fill_bank);
  assign fill_bank = ~read_bank;
  always @(posedge clk) begin
    if (rst) read_bank <= 1'b0;
    else if (swap) read_bank <= ~read_bank;
  end
endmodule
