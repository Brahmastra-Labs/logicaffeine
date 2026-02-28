; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/spectral_norm/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/spectral_norm/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [6 x i8] c"%.9f\0A\00", align 1
@.memset_pattern = private unnamed_addr constant [2 x double] [double 1.000000e+00, double 1.000000e+00], align 16

; Function Attrs: mustprogress nofree norecurse nosync nounwind ssp willreturn memory(none) uwtable(sync)
define double @A(i32 noundef %0, i32 noundef %1) local_unnamed_addr #0 {
  %3 = add nsw i32 %1, %0
  %4 = add nsw i32 %3, 1
  %5 = mul nsw i32 %4, %3
  %6 = sdiv i32 %5, 2
  %7 = add i32 %0, 1
  %8 = add i32 %7, %6
  %9 = sitofp i32 %8 to double
  %10 = fdiv double 1.000000e+00, %9
  ret double %10
}

; Function Attrs: nofree nosync nounwind ssp memory(argmem: readwrite) uwtable(sync)
define void @mul_Av(i32 noundef %0, ptr nocapture noundef readonly %1, ptr nocapture noundef writeonly %2) local_unnamed_addr #1 {
  %4 = icmp sgt i32 %0, 0
  br i1 %4, label %5, label %12

5:                                                ; preds = %3
  %6 = zext i32 %0 to i64
  %7 = shl nuw nsw i64 %6, 3
  %8 = getelementptr i8, ptr %1, i64 %7
  %9 = icmp ult i32 %0, 8
  %10 = and i64 %6, 4294967288
  %11 = icmp eq i64 %10, %6
  br label %13

12:                                               ; preds = %123, %3
  ret void

13:                                               ; preds = %123, %5
  %14 = phi i64 [ 0, %5 ], [ %16, %123 ]
  %15 = getelementptr inbounds double, ptr %2, i64 %14
  store double 0.000000e+00, ptr %15, align 8, !tbaa !6
  %16 = add nuw nsw i64 %14, 1
  %17 = trunc i64 %14 to i32
  %18 = trunc i64 %16 to i32
  br i1 %9, label %120, label %19

19:                                               ; preds = %13
  %20 = shl nuw nsw i64 %14, 3
  %21 = add nuw i64 %20, 8
  %22 = getelementptr i8, ptr %2, i64 %21
  %23 = getelementptr i8, ptr %2, i64 %20
  %24 = icmp ult ptr %23, %8
  %25 = icmp ugt ptr %22, %1
  %26 = and i1 %24, %25
  br i1 %26, label %120, label %27

27:                                               ; preds = %19
  %28 = insertelement <2 x i64> poison, i64 %14, i64 0
  %29 = shufflevector <2 x i64> %28, <2 x i64> poison, <2 x i32> zeroinitializer
  %30 = insertelement <2 x i32> poison, i32 %17, i64 0
  %31 = shufflevector <2 x i32> %30, <2 x i32> poison, <2 x i32> zeroinitializer
  %32 = insertelement <2 x i32> poison, i32 %18, i64 0
  %33 = shufflevector <2 x i32> %32, <2 x i32> poison, <2 x i32> zeroinitializer
  %34 = insertelement <2 x i32> poison, i32 %18, i64 0
  %35 = shufflevector <2 x i32> %34, <2 x i32> poison, <2 x i32> zeroinitializer
  %36 = insertelement <2 x i32> poison, i32 %18, i64 0
  %37 = shufflevector <2 x i32> %36, <2 x i32> poison, <2 x i32> zeroinitializer
  %38 = insertelement <2 x i32> poison, i32 %18, i64 0
  %39 = shufflevector <2 x i32> %38, <2 x i32> poison, <2 x i32> zeroinitializer
  %40 = add nuw i64 %14, 2
  %41 = insertelement <2 x i64> poison, i64 %40, i64 0
  %42 = shufflevector <2 x i64> %41, <2 x i64> poison, <2 x i32> zeroinitializer
  %43 = add nuw i64 %14, 4
  %44 = insertelement <2 x i64> poison, i64 %43, i64 0
  %45 = shufflevector <2 x i64> %44, <2 x i64> poison, <2 x i32> zeroinitializer
  %46 = add nuw i64 %14, 6
  %47 = insertelement <2 x i64> poison, i64 %46, i64 0
  %48 = shufflevector <2 x i64> %47, <2 x i64> poison, <2 x i32> zeroinitializer
  %49 = add i32 %17, 2
  %50 = insertelement <2 x i32> poison, i32 %49, i64 0
  %51 = shufflevector <2 x i32> %50, <2 x i32> poison, <2 x i32> zeroinitializer
  %52 = add i32 %17, 4
  %53 = insertelement <2 x i32> poison, i32 %52, i64 0
  %54 = shufflevector <2 x i32> %53, <2 x i32> poison, <2 x i32> zeroinitializer
  %55 = add i32 %17, 6
  %56 = insertelement <2 x i32> poison, i32 %55, i64 0
  %57 = shufflevector <2 x i32> %56, <2 x i32> poison, <2 x i32> zeroinitializer
  br label %58

58:                                               ; preds = %58, %27
  %59 = phi i64 [ 0, %27 ], [ %115, %58 ]
  %60 = phi <2 x i64> [ <i64 0, i64 1>, %27 ], [ %116, %58 ]
  %61 = phi double [ 0.000000e+00, %27 ], [ %114, %58 ]
  %62 = phi <2 x i32> [ <i32 0, i32 1>, %27 ], [ %117, %58 ]
  %63 = add nuw nsw <2 x i64> %60, %29
  %64 = add <2 x i64> %42, %60
  %65 = add <2 x i64> %45, %60
  %66 = add <2 x i64> %48, %60
  %67 = add nuw nsw <2 x i32> %62, %31
  %68 = add <2 x i32> %51, %62
  %69 = add <2 x i32> %54, %62
  %70 = add <2 x i32> %57, %62
  %71 = add nuw nsw <2 x i32> %67, <i32 1, i32 1>
  %72 = add nuw nsw <2 x i32> %68, <i32 1, i32 1>
  %73 = add nuw nsw <2 x i32> %69, <i32 1, i32 1>
  %74 = add nuw nsw <2 x i32> %70, <i32 1, i32 1>
  %75 = trunc <2 x i64> %63 to <2 x i32>
  %76 = trunc <2 x i64> %64 to <2 x i32>
  %77 = trunc <2 x i64> %65 to <2 x i32>
  %78 = trunc <2 x i64> %66 to <2 x i32>
  %79 = mul nsw <2 x i32> %71, %75
  %80 = mul nsw <2 x i32> %72, %76
  %81 = mul nsw <2 x i32> %73, %77
  %82 = mul nsw <2 x i32> %74, %78
  %83 = lshr <2 x i32> %79, <i32 1, i32 1>
  %84 = lshr <2 x i32> %80, <i32 1, i32 1>
  %85 = lshr <2 x i32> %81, <i32 1, i32 1>
  %86 = lshr <2 x i32> %82, <i32 1, i32 1>
  %87 = add <2 x i32> %83, %33
  %88 = add <2 x i32> %84, %35
  %89 = add <2 x i32> %85, %37
  %90 = add <2 x i32> %86, %39
  %91 = sitofp <2 x i32> %87 to <2 x double>
  %92 = sitofp <2 x i32> %88 to <2 x double>
  %93 = sitofp <2 x i32> %89 to <2 x double>
  %94 = sitofp <2 x i32> %90 to <2 x double>
  %95 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %91
  %96 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %92
  %97 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %93
  %98 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %94
  %99 = getelementptr inbounds double, ptr %1, i64 %59
  %100 = load <2 x double>, ptr %99, align 8, !tbaa !6, !alias.scope !10
  %101 = getelementptr inbounds double, ptr %99, i64 2
  %102 = load <2 x double>, ptr %101, align 8, !tbaa !6, !alias.scope !10
  %103 = getelementptr inbounds double, ptr %99, i64 4
  %104 = load <2 x double>, ptr %103, align 8, !tbaa !6, !alias.scope !10
  %105 = getelementptr inbounds double, ptr %99, i64 6
  %106 = load <2 x double>, ptr %105, align 8, !tbaa !6, !alias.scope !10
  %107 = fmul <2 x double> %95, %100
  %108 = fmul <2 x double> %96, %102
  %109 = fmul <2 x double> %97, %104
  %110 = fmul <2 x double> %98, %106
  %111 = tail call double @llvm.vector.reduce.fadd.v2f64(double %61, <2 x double> %107)
  %112 = tail call double @llvm.vector.reduce.fadd.v2f64(double %111, <2 x double> %108)
  %113 = tail call double @llvm.vector.reduce.fadd.v2f64(double %112, <2 x double> %109)
  %114 = tail call double @llvm.vector.reduce.fadd.v2f64(double %113, <2 x double> %110)
  %115 = add nuw i64 %59, 8
  %116 = add <2 x i64> %60, <i64 8, i64 8>
  %117 = add <2 x i32> %62, <i32 8, i32 8>
  %118 = icmp eq i64 %115, %10
  br i1 %118, label %119, label %58, !llvm.loop !13

119:                                              ; preds = %58
  store double %114, ptr %15, align 8, !tbaa !6
  br i1 %11, label %123, label %120

120:                                              ; preds = %19, %13, %119
  %121 = phi i64 [ 0, %19 ], [ 0, %13 ], [ %10, %119 ]
  %122 = phi double [ 0.000000e+00, %19 ], [ 0.000000e+00, %13 ], [ %114, %119 ]
  br label %125

123:                                              ; preds = %125, %119
  %124 = icmp eq i64 %16, %6
  br i1 %124, label %12, label %13, !llvm.loop !17

125:                                              ; preds = %120, %125
  %126 = phi i64 [ %141, %125 ], [ %121, %120 ]
  %127 = phi double [ %140, %125 ], [ %122, %120 ]
  %128 = trunc i64 %126 to i32
  %129 = add nuw nsw i64 %126, %14
  %130 = add nuw nsw i32 %128, %17
  %131 = add nuw nsw i32 %130, 1
  %132 = trunc i64 %129 to i32
  %133 = mul nsw i32 %131, %132
  %134 = lshr i32 %133, 1
  %135 = add i32 %134, %18
  %136 = sitofp i32 %135 to double
  %137 = fdiv double 1.000000e+00, %136
  %138 = getelementptr inbounds double, ptr %1, i64 %126
  %139 = load double, ptr %138, align 8, !tbaa !6
  %140 = tail call double @llvm.fmuladd.f64(double %137, double %139, double %127)
  store double %140, ptr %15, align 8, !tbaa !6
  %141 = add nuw nsw i64 %126, 1
  %142 = icmp eq i64 %141, %6
  br i1 %142, label %123, label %125, !llvm.loop !18
}

; Function Attrs: mustprogress nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.fmuladd.f64(double, double, double) #2

; Function Attrs: nofree nosync nounwind ssp memory(argmem: readwrite) uwtable(sync)
define void @mul_Atv(i32 noundef %0, ptr nocapture noundef readonly %1, ptr nocapture noundef writeonly %2) local_unnamed_addr #1 {
  %4 = icmp sgt i32 %0, 0
  br i1 %4, label %5, label %12

5:                                                ; preds = %3
  %6 = zext i32 %0 to i64
  %7 = shl nuw nsw i64 %6, 3
  %8 = getelementptr i8, ptr %1, i64 %7
  %9 = icmp ult i32 %0, 8
  %10 = and i64 %6, 4294967288
  %11 = icmp eq i64 %10, %6
  br label %13

12:                                               ; preds = %121, %3
  ret void

13:                                               ; preds = %121, %5
  %14 = phi i64 [ 0, %5 ], [ %122, %121 ]
  %15 = getelementptr inbounds double, ptr %2, i64 %14
  store double 0.000000e+00, ptr %15, align 8, !tbaa !6
  %16 = trunc i64 %14 to i32
  br i1 %9, label %118, label %17

17:                                               ; preds = %13
  %18 = shl nuw nsw i64 %14, 3
  %19 = add nuw i64 %18, 8
  %20 = getelementptr i8, ptr %2, i64 %19
  %21 = getelementptr i8, ptr %2, i64 %18
  %22 = icmp ult ptr %21, %8
  %23 = icmp ugt ptr %20, %1
  %24 = and i1 %22, %23
  br i1 %24, label %118, label %25

25:                                               ; preds = %17
  %26 = insertelement <2 x i64> poison, i64 %14, i64 0
  %27 = shufflevector <2 x i64> %26, <2 x i64> poison, <2 x i32> zeroinitializer
  %28 = insertelement <2 x i32> poison, i32 %16, i64 0
  %29 = shufflevector <2 x i32> %28, <2 x i32> poison, <2 x i32> zeroinitializer
  %30 = add nuw i64 %14, 2
  %31 = insertelement <2 x i64> poison, i64 %30, i64 0
  %32 = shufflevector <2 x i64> %31, <2 x i64> poison, <2 x i32> zeroinitializer
  %33 = add nuw i64 %14, 4
  %34 = insertelement <2 x i64> poison, i64 %33, i64 0
  %35 = shufflevector <2 x i64> %34, <2 x i64> poison, <2 x i32> zeroinitializer
  %36 = add nuw i64 %14, 6
  %37 = insertelement <2 x i64> poison, i64 %36, i64 0
  %38 = shufflevector <2 x i64> %37, <2 x i64> poison, <2 x i32> zeroinitializer
  %39 = add i32 %16, 2
  %40 = insertelement <2 x i32> poison, i32 %39, i64 0
  %41 = shufflevector <2 x i32> %40, <2 x i32> poison, <2 x i32> zeroinitializer
  %42 = add i32 %16, 4
  %43 = insertelement <2 x i32> poison, i32 %42, i64 0
  %44 = shufflevector <2 x i32> %43, <2 x i32> poison, <2 x i32> zeroinitializer
  %45 = add i32 %16, 6
  %46 = insertelement <2 x i32> poison, i32 %45, i64 0
  %47 = shufflevector <2 x i32> %46, <2 x i32> poison, <2 x i32> zeroinitializer
  br label %48

48:                                               ; preds = %48, %25
  %49 = phi i64 [ 0, %25 ], [ %113, %48 ]
  %50 = phi <2 x i64> [ <i64 0, i64 1>, %25 ], [ %114, %48 ]
  %51 = phi double [ 0.000000e+00, %25 ], [ %112, %48 ]
  %52 = phi <2 x i32> [ <i32 0, i32 1>, %25 ], [ %115, %48 ]
  %53 = add nuw nsw <2 x i64> %50, %27
  %54 = add <2 x i64> %32, %50
  %55 = add <2 x i64> %35, %50
  %56 = add <2 x i64> %38, %50
  %57 = add nuw nsw <2 x i32> %52, %29
  %58 = add <2 x i32> %41, %52
  %59 = add <2 x i32> %44, %52
  %60 = add <2 x i32> %47, %52
  %61 = add nuw nsw <2 x i32> %57, <i32 1, i32 1>
  %62 = add nuw nsw <2 x i32> %58, <i32 1, i32 1>
  %63 = add nuw nsw <2 x i32> %59, <i32 1, i32 1>
  %64 = add nuw nsw <2 x i32> %60, <i32 1, i32 1>
  %65 = trunc <2 x i64> %53 to <2 x i32>
  %66 = trunc <2 x i64> %54 to <2 x i32>
  %67 = trunc <2 x i64> %55 to <2 x i32>
  %68 = trunc <2 x i64> %56 to <2 x i32>
  %69 = mul nsw <2 x i32> %61, %65
  %70 = mul nsw <2 x i32> %62, %66
  %71 = mul nsw <2 x i32> %63, %67
  %72 = mul nsw <2 x i32> %64, %68
  %73 = lshr <2 x i32> %69, <i32 1, i32 1>
  %74 = lshr <2 x i32> %70, <i32 1, i32 1>
  %75 = lshr <2 x i32> %71, <i32 1, i32 1>
  %76 = lshr <2 x i32> %72, <i32 1, i32 1>
  %77 = trunc <2 x i64> %50 to <2 x i32>
  %78 = add <2 x i32> %77, <i32 1, i32 1>
  %79 = trunc <2 x i64> %50 to <2 x i32>
  %80 = add <2 x i32> %79, <i32 3, i32 3>
  %81 = trunc <2 x i64> %50 to <2 x i32>
  %82 = add <2 x i32> %81, <i32 5, i32 5>
  %83 = trunc <2 x i64> %50 to <2 x i32>
  %84 = add <2 x i32> %83, <i32 7, i32 7>
  %85 = add nuw <2 x i32> %73, %78
  %86 = add nuw <2 x i32> %74, %80
  %87 = add nuw <2 x i32> %75, %82
  %88 = add nuw <2 x i32> %76, %84
  %89 = sitofp <2 x i32> %85 to <2 x double>
  %90 = sitofp <2 x i32> %86 to <2 x double>
  %91 = sitofp <2 x i32> %87 to <2 x double>
  %92 = sitofp <2 x i32> %88 to <2 x double>
  %93 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %89
  %94 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %90
  %95 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %91
  %96 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %92
  %97 = getelementptr inbounds double, ptr %1, i64 %49
  %98 = load <2 x double>, ptr %97, align 8, !tbaa !6, !alias.scope !19
  %99 = getelementptr inbounds double, ptr %97, i64 2
  %100 = load <2 x double>, ptr %99, align 8, !tbaa !6, !alias.scope !19
  %101 = getelementptr inbounds double, ptr %97, i64 4
  %102 = load <2 x double>, ptr %101, align 8, !tbaa !6, !alias.scope !19
  %103 = getelementptr inbounds double, ptr %97, i64 6
  %104 = load <2 x double>, ptr %103, align 8, !tbaa !6, !alias.scope !19
  %105 = fmul <2 x double> %93, %98
  %106 = fmul <2 x double> %94, %100
  %107 = fmul <2 x double> %95, %102
  %108 = fmul <2 x double> %96, %104
  %109 = tail call double @llvm.vector.reduce.fadd.v2f64(double %51, <2 x double> %105)
  %110 = tail call double @llvm.vector.reduce.fadd.v2f64(double %109, <2 x double> %106)
  %111 = tail call double @llvm.vector.reduce.fadd.v2f64(double %110, <2 x double> %107)
  %112 = tail call double @llvm.vector.reduce.fadd.v2f64(double %111, <2 x double> %108)
  %113 = add nuw i64 %49, 8
  %114 = add <2 x i64> %50, <i64 8, i64 8>
  %115 = add <2 x i32> %52, <i32 8, i32 8>
  %116 = icmp eq i64 %113, %10
  br i1 %116, label %117, label %48, !llvm.loop !22

117:                                              ; preds = %48
  store double %112, ptr %15, align 8, !tbaa !6
  br i1 %11, label %121, label %118

118:                                              ; preds = %17, %13, %117
  %119 = phi i64 [ 0, %17 ], [ 0, %13 ], [ %10, %117 ]
  %120 = phi double [ 0.000000e+00, %17 ], [ 0.000000e+00, %13 ], [ %112, %117 ]
  br label %124

121:                                              ; preds = %124, %117
  %122 = add nuw nsw i64 %14, 1
  %123 = icmp eq i64 %122, %6
  br i1 %123, label %12, label %13, !llvm.loop !23

124:                                              ; preds = %118, %124
  %125 = phi i64 [ %134, %124 ], [ %119, %118 ]
  %126 = phi double [ %141, %124 ], [ %120, %118 ]
  %127 = trunc i64 %125 to i32
  %128 = add nuw nsw i64 %125, %14
  %129 = add nuw nsw i32 %127, %16
  %130 = add nuw nsw i32 %129, 1
  %131 = trunc i64 %128 to i32
  %132 = mul nsw i32 %130, %131
  %133 = lshr i32 %132, 1
  %134 = add nuw nsw i64 %125, 1
  %135 = trunc i64 %134 to i32
  %136 = add nuw i32 %133, %135
  %137 = sitofp i32 %136 to double
  %138 = fdiv double 1.000000e+00, %137
  %139 = getelementptr inbounds double, ptr %1, i64 %125
  %140 = load double, ptr %139, align 8, !tbaa !6
  %141 = tail call double @llvm.fmuladd.f64(double %138, double %140, double %126)
  store double %141, ptr %15, align 8, !tbaa !6
  %142 = icmp eq i64 %134, %6
  br i1 %142, label %121, label %124, !llvm.loop !24
}

; Function Attrs: nofree nosync nounwind ssp memory(argmem: readwrite) uwtable(sync)
define void @mul_AtAv(i32 noundef %0, ptr nocapture noundef readonly %1, ptr nocapture noundef writeonly %2, ptr nocapture noundef %3) local_unnamed_addr #1 {
  %5 = icmp sgt i32 %0, 0
  br i1 %5, label %6, label %271

6:                                                ; preds = %4
  %7 = zext i32 %0 to i64
  %8 = shl nuw nsw i64 %7, 3
  %9 = getelementptr i8, ptr %1, i64 %8
  %10 = icmp ult i32 %0, 8
  %11 = and i64 %7, 4294967288
  %12 = icmp eq i64 %11, %7
  br label %13

13:                                               ; preds = %120, %6
  %14 = phi i64 [ 0, %6 ], [ %16, %120 ]
  %15 = getelementptr inbounds double, ptr %3, i64 %14
  store double 0.000000e+00, ptr %15, align 8, !tbaa !6
  %16 = add nuw nsw i64 %14, 1
  %17 = trunc i64 %14 to i32
  %18 = trunc i64 %16 to i32
  %19 = add i32 %17, 1
  br i1 %10, label %117, label %20

20:                                               ; preds = %13
  %21 = shl nuw nsw i64 %14, 3
  %22 = add nuw i64 %21, 8
  %23 = getelementptr i8, ptr %3, i64 %22
  %24 = getelementptr i8, ptr %3, i64 %21
  %25 = icmp ult ptr %24, %9
  %26 = icmp ugt ptr %23, %1
  %27 = and i1 %25, %26
  br i1 %27, label %117, label %28

28:                                               ; preds = %20
  %29 = insertelement <2 x i64> poison, i64 %14, i64 0
  %30 = shufflevector <2 x i64> %29, <2 x i64> poison, <2 x i32> zeroinitializer
  %31 = insertelement <2 x i32> poison, i32 %19, i64 0
  %32 = shufflevector <2 x i32> %31, <2 x i32> poison, <2 x i32> zeroinitializer
  %33 = insertelement <2 x i32> poison, i32 %18, i64 0
  %34 = shufflevector <2 x i32> %33, <2 x i32> poison, <2 x i32> zeroinitializer
  %35 = insertelement <2 x i32> poison, i32 %18, i64 0
  %36 = shufflevector <2 x i32> %35, <2 x i32> poison, <2 x i32> zeroinitializer
  %37 = insertelement <2 x i32> poison, i32 %18, i64 0
  %38 = shufflevector <2 x i32> %37, <2 x i32> poison, <2 x i32> zeroinitializer
  %39 = insertelement <2 x i32> poison, i32 %18, i64 0
  %40 = shufflevector <2 x i32> %39, <2 x i32> poison, <2 x i32> zeroinitializer
  %41 = add nuw i64 %14, 2
  %42 = insertelement <2 x i64> poison, i64 %41, i64 0
  %43 = shufflevector <2 x i64> %42, <2 x i64> poison, <2 x i32> zeroinitializer
  %44 = add nuw i64 %14, 4
  %45 = insertelement <2 x i64> poison, i64 %44, i64 0
  %46 = shufflevector <2 x i64> %45, <2 x i64> poison, <2 x i32> zeroinitializer
  %47 = add nuw i64 %14, 6
  %48 = insertelement <2 x i64> poison, i64 %47, i64 0
  %49 = shufflevector <2 x i64> %48, <2 x i64> poison, <2 x i32> zeroinitializer
  %50 = add i32 %17, 3
  %51 = insertelement <2 x i32> poison, i32 %50, i64 0
  %52 = shufflevector <2 x i32> %51, <2 x i32> poison, <2 x i32> zeroinitializer
  %53 = add i32 %17, 5
  %54 = insertelement <2 x i32> poison, i32 %53, i64 0
  %55 = shufflevector <2 x i32> %54, <2 x i32> poison, <2 x i32> zeroinitializer
  %56 = add i32 %17, 7
  %57 = insertelement <2 x i32> poison, i32 %56, i64 0
  %58 = shufflevector <2 x i32> %57, <2 x i32> poison, <2 x i32> zeroinitializer
  br label %59

59:                                               ; preds = %59, %28
  %60 = phi i64 [ 0, %28 ], [ %112, %59 ]
  %61 = phi <2 x i64> [ <i64 0, i64 1>, %28 ], [ %113, %59 ]
  %62 = phi double [ 0.000000e+00, %28 ], [ %111, %59 ]
  %63 = phi <2 x i32> [ <i32 0, i32 1>, %28 ], [ %114, %59 ]
  %64 = add nuw nsw <2 x i64> %61, %30
  %65 = add <2 x i64> %43, %61
  %66 = add <2 x i64> %46, %61
  %67 = add <2 x i64> %49, %61
  %68 = add <2 x i32> %32, %63
  %69 = add <2 x i32> %52, %63
  %70 = add <2 x i32> %55, %63
  %71 = add <2 x i32> %58, %63
  %72 = trunc <2 x i64> %64 to <2 x i32>
  %73 = trunc <2 x i64> %65 to <2 x i32>
  %74 = trunc <2 x i64> %66 to <2 x i32>
  %75 = trunc <2 x i64> %67 to <2 x i32>
  %76 = mul nsw <2 x i32> %68, %72
  %77 = mul nsw <2 x i32> %69, %73
  %78 = mul nsw <2 x i32> %70, %74
  %79 = mul nsw <2 x i32> %71, %75
  %80 = lshr <2 x i32> %76, <i32 1, i32 1>
  %81 = lshr <2 x i32> %77, <i32 1, i32 1>
  %82 = lshr <2 x i32> %78, <i32 1, i32 1>
  %83 = lshr <2 x i32> %79, <i32 1, i32 1>
  %84 = add <2 x i32> %80, %34
  %85 = add <2 x i32> %81, %36
  %86 = add <2 x i32> %82, %38
  %87 = add <2 x i32> %83, %40
  %88 = sitofp <2 x i32> %84 to <2 x double>
  %89 = sitofp <2 x i32> %85 to <2 x double>
  %90 = sitofp <2 x i32> %86 to <2 x double>
  %91 = sitofp <2 x i32> %87 to <2 x double>
  %92 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %88
  %93 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %89
  %94 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %90
  %95 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %91
  %96 = getelementptr inbounds double, ptr %1, i64 %60
  %97 = load <2 x double>, ptr %96, align 8, !tbaa !6, !alias.scope !25
  %98 = getelementptr inbounds double, ptr %96, i64 2
  %99 = load <2 x double>, ptr %98, align 8, !tbaa !6, !alias.scope !25
  %100 = getelementptr inbounds double, ptr %96, i64 4
  %101 = load <2 x double>, ptr %100, align 8, !tbaa !6, !alias.scope !25
  %102 = getelementptr inbounds double, ptr %96, i64 6
  %103 = load <2 x double>, ptr %102, align 8, !tbaa !6, !alias.scope !25
  %104 = fmul <2 x double> %92, %97
  %105 = fmul <2 x double> %93, %99
  %106 = fmul <2 x double> %94, %101
  %107 = fmul <2 x double> %95, %103
  %108 = tail call double @llvm.vector.reduce.fadd.v2f64(double %62, <2 x double> %104)
  %109 = tail call double @llvm.vector.reduce.fadd.v2f64(double %108, <2 x double> %105)
  %110 = tail call double @llvm.vector.reduce.fadd.v2f64(double %109, <2 x double> %106)
  %111 = tail call double @llvm.vector.reduce.fadd.v2f64(double %110, <2 x double> %107)
  %112 = add nuw i64 %60, 8
  %113 = add <2 x i64> %61, <i64 8, i64 8>
  %114 = add <2 x i32> %63, <i32 8, i32 8>
  %115 = icmp eq i64 %112, %11
  br i1 %115, label %116, label %59, !llvm.loop !28

116:                                              ; preds = %59
  store double %111, ptr %15, align 8, !tbaa !6
  br i1 %12, label %120, label %117

117:                                              ; preds = %20, %13, %116
  %118 = phi i64 [ 0, %20 ], [ 0, %13 ], [ %11, %116 ]
  %119 = phi double [ 0.000000e+00, %20 ], [ 0.000000e+00, %13 ], [ %111, %116 ]
  br label %128

120:                                              ; preds = %128, %116
  %121 = icmp eq i64 %16, %7
  br i1 %121, label %122, label %13, !llvm.loop !17

122:                                              ; preds = %120
  %123 = shl nuw nsw i64 %7, 3
  %124 = getelementptr i8, ptr %3, i64 %123
  %125 = icmp ult i32 %0, 8
  %126 = and i64 %7, 4294967288
  %127 = icmp eq i64 %126, %7
  br label %145

128:                                              ; preds = %117, %128
  %129 = phi i64 [ %143, %128 ], [ %118, %117 ]
  %130 = phi double [ %142, %128 ], [ %119, %117 ]
  %131 = trunc i64 %129 to i32
  %132 = add nuw nsw i64 %129, %14
  %133 = add i32 %19, %131
  %134 = trunc i64 %132 to i32
  %135 = mul nsw i32 %133, %134
  %136 = lshr i32 %135, 1
  %137 = add i32 %136, %18
  %138 = sitofp i32 %137 to double
  %139 = fdiv double 1.000000e+00, %138
  %140 = getelementptr inbounds double, ptr %1, i64 %129
  %141 = load double, ptr %140, align 8, !tbaa !6
  %142 = tail call double @llvm.fmuladd.f64(double %139, double %141, double %130)
  store double %142, ptr %15, align 8, !tbaa !6
  %143 = add nuw nsw i64 %129, 1
  %144 = icmp eq i64 %143, %7
  br i1 %144, label %120, label %128, !llvm.loop !29

145:                                              ; preds = %122, %250
  %146 = phi i64 [ %251, %250 ], [ 0, %122 ]
  %147 = getelementptr inbounds double, ptr %2, i64 %146
  store double 0.000000e+00, ptr %147, align 8, !tbaa !6
  %148 = trunc i64 %146 to i32
  %149 = add i32 %148, 1
  br i1 %125, label %247, label %150

150:                                              ; preds = %145
  %151 = shl nuw nsw i64 %146, 3
  %152 = add nuw i64 %151, 8
  %153 = getelementptr i8, ptr %2, i64 %152
  %154 = getelementptr i8, ptr %2, i64 %151
  %155 = icmp ult ptr %154, %124
  %156 = icmp ugt ptr %153, %3
  %157 = and i1 %155, %156
  br i1 %157, label %247, label %158

158:                                              ; preds = %150
  %159 = insertelement <2 x i64> poison, i64 %146, i64 0
  %160 = shufflevector <2 x i64> %159, <2 x i64> poison, <2 x i32> zeroinitializer
  %161 = insertelement <2 x i32> poison, i32 %149, i64 0
  %162 = shufflevector <2 x i32> %161, <2 x i32> poison, <2 x i32> zeroinitializer
  %163 = add nuw i64 %146, 2
  %164 = insertelement <2 x i64> poison, i64 %163, i64 0
  %165 = shufflevector <2 x i64> %164, <2 x i64> poison, <2 x i32> zeroinitializer
  %166 = add nuw i64 %146, 4
  %167 = insertelement <2 x i64> poison, i64 %166, i64 0
  %168 = shufflevector <2 x i64> %167, <2 x i64> poison, <2 x i32> zeroinitializer
  %169 = add nuw i64 %146, 6
  %170 = insertelement <2 x i64> poison, i64 %169, i64 0
  %171 = shufflevector <2 x i64> %170, <2 x i64> poison, <2 x i32> zeroinitializer
  %172 = add i32 %148, 3
  %173 = insertelement <2 x i32> poison, i32 %172, i64 0
  %174 = shufflevector <2 x i32> %173, <2 x i32> poison, <2 x i32> zeroinitializer
  %175 = add i32 %148, 5
  %176 = insertelement <2 x i32> poison, i32 %175, i64 0
  %177 = shufflevector <2 x i32> %176, <2 x i32> poison, <2 x i32> zeroinitializer
  %178 = add i32 %148, 7
  %179 = insertelement <2 x i32> poison, i32 %178, i64 0
  %180 = shufflevector <2 x i32> %179, <2 x i32> poison, <2 x i32> zeroinitializer
  br label %181

181:                                              ; preds = %181, %158
  %182 = phi i64 [ 0, %158 ], [ %242, %181 ]
  %183 = phi <2 x i64> [ <i64 0, i64 1>, %158 ], [ %243, %181 ]
  %184 = phi double [ 0.000000e+00, %158 ], [ %241, %181 ]
  %185 = phi <2 x i32> [ <i32 0, i32 1>, %158 ], [ %244, %181 ]
  %186 = add nuw nsw <2 x i64> %183, %160
  %187 = add <2 x i64> %165, %183
  %188 = add <2 x i64> %168, %183
  %189 = add <2 x i64> %171, %183
  %190 = add <2 x i32> %162, %185
  %191 = add <2 x i32> %174, %185
  %192 = add <2 x i32> %177, %185
  %193 = add <2 x i32> %180, %185
  %194 = trunc <2 x i64> %186 to <2 x i32>
  %195 = trunc <2 x i64> %187 to <2 x i32>
  %196 = trunc <2 x i64> %188 to <2 x i32>
  %197 = trunc <2 x i64> %189 to <2 x i32>
  %198 = mul nsw <2 x i32> %190, %194
  %199 = mul nsw <2 x i32> %191, %195
  %200 = mul nsw <2 x i32> %192, %196
  %201 = mul nsw <2 x i32> %193, %197
  %202 = lshr <2 x i32> %198, <i32 1, i32 1>
  %203 = lshr <2 x i32> %199, <i32 1, i32 1>
  %204 = lshr <2 x i32> %200, <i32 1, i32 1>
  %205 = lshr <2 x i32> %201, <i32 1, i32 1>
  %206 = trunc <2 x i64> %183 to <2 x i32>
  %207 = add <2 x i32> %206, <i32 1, i32 1>
  %208 = trunc <2 x i64> %183 to <2 x i32>
  %209 = add <2 x i32> %208, <i32 3, i32 3>
  %210 = trunc <2 x i64> %183 to <2 x i32>
  %211 = add <2 x i32> %210, <i32 5, i32 5>
  %212 = trunc <2 x i64> %183 to <2 x i32>
  %213 = add <2 x i32> %212, <i32 7, i32 7>
  %214 = add nuw <2 x i32> %202, %207
  %215 = add nuw <2 x i32> %203, %209
  %216 = add nuw <2 x i32> %204, %211
  %217 = add nuw <2 x i32> %205, %213
  %218 = sitofp <2 x i32> %214 to <2 x double>
  %219 = sitofp <2 x i32> %215 to <2 x double>
  %220 = sitofp <2 x i32> %216 to <2 x double>
  %221 = sitofp <2 x i32> %217 to <2 x double>
  %222 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %218
  %223 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %219
  %224 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %220
  %225 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %221
  %226 = getelementptr inbounds double, ptr %3, i64 %182
  %227 = load <2 x double>, ptr %226, align 8, !tbaa !6, !alias.scope !30
  %228 = getelementptr inbounds double, ptr %226, i64 2
  %229 = load <2 x double>, ptr %228, align 8, !tbaa !6, !alias.scope !30
  %230 = getelementptr inbounds double, ptr %226, i64 4
  %231 = load <2 x double>, ptr %230, align 8, !tbaa !6, !alias.scope !30
  %232 = getelementptr inbounds double, ptr %226, i64 6
  %233 = load <2 x double>, ptr %232, align 8, !tbaa !6, !alias.scope !30
  %234 = fmul <2 x double> %222, %227
  %235 = fmul <2 x double> %223, %229
  %236 = fmul <2 x double> %224, %231
  %237 = fmul <2 x double> %225, %233
  %238 = tail call double @llvm.vector.reduce.fadd.v2f64(double %184, <2 x double> %234)
  %239 = tail call double @llvm.vector.reduce.fadd.v2f64(double %238, <2 x double> %235)
  %240 = tail call double @llvm.vector.reduce.fadd.v2f64(double %239, <2 x double> %236)
  %241 = tail call double @llvm.vector.reduce.fadd.v2f64(double %240, <2 x double> %237)
  %242 = add nuw i64 %182, 8
  %243 = add <2 x i64> %183, <i64 8, i64 8>
  %244 = add <2 x i32> %185, <i32 8, i32 8>
  %245 = icmp eq i64 %242, %126
  br i1 %245, label %246, label %181, !llvm.loop !33

246:                                              ; preds = %181
  store double %241, ptr %147, align 8, !tbaa !6
  br i1 %127, label %250, label %247

247:                                              ; preds = %150, %145, %246
  %248 = phi i64 [ 0, %150 ], [ 0, %145 ], [ %126, %246 ]
  %249 = phi double [ 0.000000e+00, %150 ], [ 0.000000e+00, %145 ], [ %241, %246 ]
  br label %253

250:                                              ; preds = %253, %246
  %251 = add nuw nsw i64 %146, 1
  %252 = icmp eq i64 %251, %7
  br i1 %252, label %271, label %145, !llvm.loop !23

253:                                              ; preds = %247, %253
  %254 = phi i64 [ %262, %253 ], [ %248, %247 ]
  %255 = phi double [ %269, %253 ], [ %249, %247 ]
  %256 = trunc i64 %254 to i32
  %257 = add nuw nsw i64 %254, %146
  %258 = add i32 %149, %256
  %259 = trunc i64 %257 to i32
  %260 = mul nsw i32 %258, %259
  %261 = lshr i32 %260, 1
  %262 = add nuw nsw i64 %254, 1
  %263 = trunc i64 %262 to i32
  %264 = add nuw i32 %261, %263
  %265 = sitofp i32 %264 to double
  %266 = fdiv double 1.000000e+00, %265
  %267 = getelementptr inbounds double, ptr %3, i64 %254
  %268 = load double, ptr %267, align 8, !tbaa !6
  %269 = tail call double @llvm.fmuladd.f64(double %266, double %268, double %255)
  store double %269, ptr %147, align 8, !tbaa !6
  %270 = icmp eq i64 %262, %7
  br i1 %270, label %250, label %253, !llvm.loop !34

271:                                              ; preds = %250, %4
  ret void
}

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #3 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %588, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !35
  %7 = tail call i32 @atoi(ptr nocapture noundef %6)
  %8 = sext i32 %7 to i64
  %9 = shl nsw i64 %8, 3
  %10 = tail call ptr @malloc(i64 noundef %9) #10
  %11 = tail call ptr @malloc(i64 noundef %9) #10
  %12 = tail call ptr @malloc(i64 noundef %9) #10
  %13 = icmp sgt i32 %7, 0
  %14 = zext i32 %7 to i64
  br i1 %13, label %15, label %17

15:                                               ; preds = %4
  %16 = shl nuw nsw i64 %14, 3
  tail call void @memset_pattern16(ptr %10, ptr nonnull @.memset_pattern, i64 %16), !tbaa !6
  br label %17

17:                                               ; preds = %4, %15
  %18 = icmp ult i32 %7, 8
  %19 = and i64 %14, 4294967288
  %20 = icmp eq i64 %19, %14
  %21 = icmp ult i32 %7, 8
  %22 = and i64 %14, 4294967288
  %23 = icmp eq i64 %22, %14
  %24 = icmp ult i32 %7, 8
  %25 = and i64 %14, 4294967288
  %26 = icmp eq i64 %25, %14
  %27 = icmp ult i32 %7, 8
  %28 = and i64 %14, 4294967288
  %29 = icmp eq i64 %28, %14
  br label %82

30:                                               ; preds = %560
  br i1 %13, label %31, label %572

31:                                               ; preds = %30
  %32 = icmp ult i32 %7, 4
  br i1 %32, label %78, label %33

33:                                               ; preds = %31
  %34 = and i64 %14, 4294967292
  br label %35

35:                                               ; preds = %35, %33
  %36 = phi i64 [ 0, %33 ], [ %74, %35 ]
  %37 = phi double [ 0.000000e+00, %33 ], [ %69, %35 ]
  %38 = phi double [ 0.000000e+00, %33 ], [ %73, %35 ]
  %39 = or i64 %36, 1
  %40 = or i64 %36, 2
  %41 = or i64 %36, 3
  %42 = getelementptr inbounds double, ptr %10, i64 %36
  %43 = getelementptr inbounds double, ptr %10, i64 %39
  %44 = getelementptr inbounds double, ptr %10, i64 %40
  %45 = getelementptr inbounds double, ptr %10, i64 %41
  %46 = load double, ptr %42, align 8, !tbaa !6
  %47 = load double, ptr %43, align 8, !tbaa !6
  %48 = load double, ptr %44, align 8, !tbaa !6
  %49 = load double, ptr %45, align 8, !tbaa !6
  %50 = getelementptr inbounds double, ptr %11, i64 %36
  %51 = getelementptr inbounds double, ptr %11, i64 %39
  %52 = getelementptr inbounds double, ptr %11, i64 %40
  %53 = getelementptr inbounds double, ptr %11, i64 %41
  %54 = load double, ptr %50, align 8, !tbaa !6
  %55 = load double, ptr %51, align 8, !tbaa !6
  %56 = load double, ptr %52, align 8, !tbaa !6
  %57 = load double, ptr %53, align 8, !tbaa !6
  %58 = fmul double %46, %54
  %59 = fmul double %47, %55
  %60 = fmul double %48, %56
  %61 = fmul double %49, %57
  %62 = fmul double %54, %54
  %63 = fmul double %55, %55
  %64 = fmul double %56, %56
  %65 = fmul double %57, %57
  %66 = fadd double %37, %62
  %67 = fadd double %66, %63
  %68 = fadd double %67, %64
  %69 = fadd double %68, %65
  %70 = fadd double %38, %58
  %71 = fadd double %70, %59
  %72 = fadd double %71, %60
  %73 = fadd double %72, %61
  %74 = add nuw i64 %36, 4
  %75 = icmp eq i64 %74, %34
  br i1 %75, label %76, label %35, !llvm.loop !37

76:                                               ; preds = %35
  %77 = icmp eq i64 %34, %14
  br i1 %77, label %568, label %78

78:                                               ; preds = %31, %76
  %79 = phi i64 [ 0, %31 ], [ %34, %76 ]
  %80 = phi double [ 0.000000e+00, %31 ], [ %69, %76 ]
  %81 = phi double [ 0.000000e+00, %31 ], [ %73, %76 ]
  br label %576

82:                                               ; preds = %563, %17
  %83 = phi i32 [ 0, %17 ], [ %564, %563 ]
  br i1 %13, label %84, label %565

84:                                               ; preds = %82, %183
  %85 = phi i64 [ %87, %183 ], [ 0, %82 ]
  %86 = getelementptr inbounds double, ptr %12, i64 %85
  %87 = add nuw nsw i64 %85, 1
  %88 = trunc i64 %85 to i32
  %89 = trunc i64 %87 to i32
  %90 = add i32 %88, 1
  br i1 %18, label %180, label %91

91:                                               ; preds = %84
  %92 = insertelement <2 x i64> poison, i64 %85, i64 0
  %93 = shufflevector <2 x i64> %92, <2 x i64> poison, <2 x i32> zeroinitializer
  %94 = insertelement <2 x i32> poison, i32 %90, i64 0
  %95 = shufflevector <2 x i32> %94, <2 x i32> poison, <2 x i32> zeroinitializer
  %96 = insertelement <2 x i32> poison, i32 %89, i64 0
  %97 = shufflevector <2 x i32> %96, <2 x i32> poison, <2 x i32> zeroinitializer
  %98 = insertelement <2 x i32> poison, i32 %89, i64 0
  %99 = shufflevector <2 x i32> %98, <2 x i32> poison, <2 x i32> zeroinitializer
  %100 = insertelement <2 x i32> poison, i32 %89, i64 0
  %101 = shufflevector <2 x i32> %100, <2 x i32> poison, <2 x i32> zeroinitializer
  %102 = insertelement <2 x i32> poison, i32 %89, i64 0
  %103 = shufflevector <2 x i32> %102, <2 x i32> poison, <2 x i32> zeroinitializer
  %104 = add nuw i64 %85, 2
  %105 = insertelement <2 x i64> poison, i64 %104, i64 0
  %106 = shufflevector <2 x i64> %105, <2 x i64> poison, <2 x i32> zeroinitializer
  %107 = add nuw i64 %85, 4
  %108 = insertelement <2 x i64> poison, i64 %107, i64 0
  %109 = shufflevector <2 x i64> %108, <2 x i64> poison, <2 x i32> zeroinitializer
  %110 = add nuw i64 %85, 6
  %111 = insertelement <2 x i64> poison, i64 %110, i64 0
  %112 = shufflevector <2 x i64> %111, <2 x i64> poison, <2 x i32> zeroinitializer
  %113 = add i32 %88, 3
  %114 = insertelement <2 x i32> poison, i32 %113, i64 0
  %115 = shufflevector <2 x i32> %114, <2 x i32> poison, <2 x i32> zeroinitializer
  %116 = add i32 %88, 5
  %117 = insertelement <2 x i32> poison, i32 %116, i64 0
  %118 = shufflevector <2 x i32> %117, <2 x i32> poison, <2 x i32> zeroinitializer
  %119 = add i32 %88, 7
  %120 = insertelement <2 x i32> poison, i32 %119, i64 0
  %121 = shufflevector <2 x i32> %120, <2 x i32> poison, <2 x i32> zeroinitializer
  br label %122

122:                                              ; preds = %122, %91
  %123 = phi i64 [ 0, %91 ], [ %175, %122 ]
  %124 = phi <2 x i64> [ <i64 0, i64 1>, %91 ], [ %176, %122 ]
  %125 = phi double [ 0.000000e+00, %91 ], [ %174, %122 ]
  %126 = phi <2 x i32> [ <i32 0, i32 1>, %91 ], [ %177, %122 ]
  %127 = add nuw nsw <2 x i64> %124, %93
  %128 = add <2 x i64> %106, %124
  %129 = add <2 x i64> %109, %124
  %130 = add <2 x i64> %112, %124
  %131 = add <2 x i32> %95, %126
  %132 = add <2 x i32> %115, %126
  %133 = add <2 x i32> %118, %126
  %134 = add <2 x i32> %121, %126
  %135 = trunc <2 x i64> %127 to <2 x i32>
  %136 = trunc <2 x i64> %128 to <2 x i32>
  %137 = trunc <2 x i64> %129 to <2 x i32>
  %138 = trunc <2 x i64> %130 to <2 x i32>
  %139 = mul nsw <2 x i32> %131, %135
  %140 = mul nsw <2 x i32> %132, %136
  %141 = mul nsw <2 x i32> %133, %137
  %142 = mul nsw <2 x i32> %134, %138
  %143 = lshr <2 x i32> %139, <i32 1, i32 1>
  %144 = lshr <2 x i32> %140, <i32 1, i32 1>
  %145 = lshr <2 x i32> %141, <i32 1, i32 1>
  %146 = lshr <2 x i32> %142, <i32 1, i32 1>
  %147 = add <2 x i32> %143, %97
  %148 = add <2 x i32> %144, %99
  %149 = add <2 x i32> %145, %101
  %150 = add <2 x i32> %146, %103
  %151 = sitofp <2 x i32> %147 to <2 x double>
  %152 = sitofp <2 x i32> %148 to <2 x double>
  %153 = sitofp <2 x i32> %149 to <2 x double>
  %154 = sitofp <2 x i32> %150 to <2 x double>
  %155 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %151
  %156 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %152
  %157 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %153
  %158 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %154
  %159 = getelementptr inbounds double, ptr %10, i64 %123
  %160 = load <2 x double>, ptr %159, align 8, !tbaa !6
  %161 = getelementptr inbounds double, ptr %159, i64 2
  %162 = load <2 x double>, ptr %161, align 8, !tbaa !6
  %163 = getelementptr inbounds double, ptr %159, i64 4
  %164 = load <2 x double>, ptr %163, align 8, !tbaa !6
  %165 = getelementptr inbounds double, ptr %159, i64 6
  %166 = load <2 x double>, ptr %165, align 8, !tbaa !6
  %167 = fmul <2 x double> %155, %160
  %168 = fmul <2 x double> %156, %162
  %169 = fmul <2 x double> %157, %164
  %170 = fmul <2 x double> %158, %166
  %171 = tail call double @llvm.vector.reduce.fadd.v2f64(double %125, <2 x double> %167)
  %172 = tail call double @llvm.vector.reduce.fadd.v2f64(double %171, <2 x double> %168)
  %173 = tail call double @llvm.vector.reduce.fadd.v2f64(double %172, <2 x double> %169)
  %174 = tail call double @llvm.vector.reduce.fadd.v2f64(double %173, <2 x double> %170)
  %175 = add nuw i64 %123, 8
  %176 = add <2 x i64> %124, <i64 8, i64 8>
  %177 = add <2 x i32> %126, <i32 8, i32 8>
  %178 = icmp eq i64 %175, %19
  br i1 %178, label %179, label %122, !llvm.loop !38

179:                                              ; preds = %122
  br i1 %20, label %183, label %180

180:                                              ; preds = %84, %179
  %181 = phi i64 [ 0, %84 ], [ %19, %179 ]
  %182 = phi double [ 0.000000e+00, %84 ], [ %174, %179 ]
  br label %186

183:                                              ; preds = %186, %179
  %184 = phi double [ %174, %179 ], [ %200, %186 ]
  store double %184, ptr %86, align 8, !tbaa !6
  %185 = icmp eq i64 %87, %14
  br i1 %185, label %203, label %84, !llvm.loop !17

186:                                              ; preds = %180, %186
  %187 = phi i64 [ %201, %186 ], [ %181, %180 ]
  %188 = phi double [ %200, %186 ], [ %182, %180 ]
  %189 = trunc i64 %187 to i32
  %190 = add nuw nsw i64 %187, %85
  %191 = add i32 %90, %189
  %192 = trunc i64 %190 to i32
  %193 = mul nsw i32 %191, %192
  %194 = lshr i32 %193, 1
  %195 = add i32 %194, %89
  %196 = sitofp i32 %195 to double
  %197 = fdiv double 1.000000e+00, %196
  %198 = getelementptr inbounds double, ptr %10, i64 %187
  %199 = load double, ptr %198, align 8, !tbaa !6
  %200 = tail call double @llvm.fmuladd.f64(double %197, double %199, double %188)
  %201 = add nuw nsw i64 %187, 1
  %202 = icmp eq i64 %201, %14
  br i1 %202, label %183, label %186, !llvm.loop !39

203:                                              ; preds = %183, %300
  %204 = phi i64 [ %302, %300 ], [ 0, %183 ]
  %205 = getelementptr inbounds double, ptr %11, i64 %204
  %206 = trunc i64 %204 to i32
  %207 = add i32 %206, 1
  br i1 %21, label %297, label %208

208:                                              ; preds = %203
  %209 = insertelement <2 x i64> poison, i64 %204, i64 0
  %210 = shufflevector <2 x i64> %209, <2 x i64> poison, <2 x i32> zeroinitializer
  %211 = insertelement <2 x i32> poison, i32 %207, i64 0
  %212 = shufflevector <2 x i32> %211, <2 x i32> poison, <2 x i32> zeroinitializer
  %213 = add nuw i64 %204, 2
  %214 = insertelement <2 x i64> poison, i64 %213, i64 0
  %215 = shufflevector <2 x i64> %214, <2 x i64> poison, <2 x i32> zeroinitializer
  %216 = add nuw i64 %204, 4
  %217 = insertelement <2 x i64> poison, i64 %216, i64 0
  %218 = shufflevector <2 x i64> %217, <2 x i64> poison, <2 x i32> zeroinitializer
  %219 = add nuw i64 %204, 6
  %220 = insertelement <2 x i64> poison, i64 %219, i64 0
  %221 = shufflevector <2 x i64> %220, <2 x i64> poison, <2 x i32> zeroinitializer
  %222 = add i32 %206, 3
  %223 = insertelement <2 x i32> poison, i32 %222, i64 0
  %224 = shufflevector <2 x i32> %223, <2 x i32> poison, <2 x i32> zeroinitializer
  %225 = add i32 %206, 5
  %226 = insertelement <2 x i32> poison, i32 %225, i64 0
  %227 = shufflevector <2 x i32> %226, <2 x i32> poison, <2 x i32> zeroinitializer
  %228 = add i32 %206, 7
  %229 = insertelement <2 x i32> poison, i32 %228, i64 0
  %230 = shufflevector <2 x i32> %229, <2 x i32> poison, <2 x i32> zeroinitializer
  br label %231

231:                                              ; preds = %231, %208
  %232 = phi i64 [ 0, %208 ], [ %292, %231 ]
  %233 = phi <2 x i64> [ <i64 0, i64 1>, %208 ], [ %293, %231 ]
  %234 = phi double [ 0.000000e+00, %208 ], [ %291, %231 ]
  %235 = phi <2 x i32> [ <i32 0, i32 1>, %208 ], [ %294, %231 ]
  %236 = add nuw nsw <2 x i64> %233, %210
  %237 = add <2 x i64> %215, %233
  %238 = add <2 x i64> %218, %233
  %239 = add <2 x i64> %221, %233
  %240 = add <2 x i32> %212, %235
  %241 = add <2 x i32> %224, %235
  %242 = add <2 x i32> %227, %235
  %243 = add <2 x i32> %230, %235
  %244 = trunc <2 x i64> %236 to <2 x i32>
  %245 = trunc <2 x i64> %237 to <2 x i32>
  %246 = trunc <2 x i64> %238 to <2 x i32>
  %247 = trunc <2 x i64> %239 to <2 x i32>
  %248 = mul nsw <2 x i32> %240, %244
  %249 = mul nsw <2 x i32> %241, %245
  %250 = mul nsw <2 x i32> %242, %246
  %251 = mul nsw <2 x i32> %243, %247
  %252 = lshr <2 x i32> %248, <i32 1, i32 1>
  %253 = lshr <2 x i32> %249, <i32 1, i32 1>
  %254 = lshr <2 x i32> %250, <i32 1, i32 1>
  %255 = lshr <2 x i32> %251, <i32 1, i32 1>
  %256 = trunc <2 x i64> %233 to <2 x i32>
  %257 = add <2 x i32> %256, <i32 1, i32 1>
  %258 = trunc <2 x i64> %233 to <2 x i32>
  %259 = add <2 x i32> %258, <i32 3, i32 3>
  %260 = trunc <2 x i64> %233 to <2 x i32>
  %261 = add <2 x i32> %260, <i32 5, i32 5>
  %262 = trunc <2 x i64> %233 to <2 x i32>
  %263 = add <2 x i32> %262, <i32 7, i32 7>
  %264 = add nuw <2 x i32> %252, %257
  %265 = add nuw <2 x i32> %253, %259
  %266 = add nuw <2 x i32> %254, %261
  %267 = add nuw <2 x i32> %255, %263
  %268 = sitofp <2 x i32> %264 to <2 x double>
  %269 = sitofp <2 x i32> %265 to <2 x double>
  %270 = sitofp <2 x i32> %266 to <2 x double>
  %271 = sitofp <2 x i32> %267 to <2 x double>
  %272 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %268
  %273 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %269
  %274 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %270
  %275 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %271
  %276 = getelementptr inbounds double, ptr %12, i64 %232
  %277 = load <2 x double>, ptr %276, align 8, !tbaa !6
  %278 = getelementptr inbounds double, ptr %276, i64 2
  %279 = load <2 x double>, ptr %278, align 8, !tbaa !6
  %280 = getelementptr inbounds double, ptr %276, i64 4
  %281 = load <2 x double>, ptr %280, align 8, !tbaa !6
  %282 = getelementptr inbounds double, ptr %276, i64 6
  %283 = load <2 x double>, ptr %282, align 8, !tbaa !6
  %284 = fmul <2 x double> %272, %277
  %285 = fmul <2 x double> %273, %279
  %286 = fmul <2 x double> %274, %281
  %287 = fmul <2 x double> %275, %283
  %288 = tail call double @llvm.vector.reduce.fadd.v2f64(double %234, <2 x double> %284)
  %289 = tail call double @llvm.vector.reduce.fadd.v2f64(double %288, <2 x double> %285)
  %290 = tail call double @llvm.vector.reduce.fadd.v2f64(double %289, <2 x double> %286)
  %291 = tail call double @llvm.vector.reduce.fadd.v2f64(double %290, <2 x double> %287)
  %292 = add nuw i64 %232, 8
  %293 = add <2 x i64> %233, <i64 8, i64 8>
  %294 = add <2 x i32> %235, <i32 8, i32 8>
  %295 = icmp eq i64 %292, %22
  br i1 %295, label %296, label %231, !llvm.loop !40

296:                                              ; preds = %231
  br i1 %23, label %300, label %297

297:                                              ; preds = %203, %296
  %298 = phi i64 [ 0, %203 ], [ %22, %296 ]
  %299 = phi double [ 0.000000e+00, %203 ], [ %291, %296 ]
  br label %304

300:                                              ; preds = %304, %296
  %301 = phi double [ %291, %296 ], [ %320, %304 ]
  store double %301, ptr %205, align 8, !tbaa !6
  %302 = add nuw nsw i64 %204, 1
  %303 = icmp eq i64 %302, %14
  br i1 %303, label %322, label %203, !llvm.loop !23

304:                                              ; preds = %297, %304
  %305 = phi i64 [ %313, %304 ], [ %298, %297 ]
  %306 = phi double [ %320, %304 ], [ %299, %297 ]
  %307 = trunc i64 %305 to i32
  %308 = add nuw nsw i64 %305, %204
  %309 = add i32 %207, %307
  %310 = trunc i64 %308 to i32
  %311 = mul nsw i32 %309, %310
  %312 = lshr i32 %311, 1
  %313 = add nuw nsw i64 %305, 1
  %314 = trunc i64 %313 to i32
  %315 = add nuw i32 %312, %314
  %316 = sitofp i32 %315 to double
  %317 = fdiv double 1.000000e+00, %316
  %318 = getelementptr inbounds double, ptr %12, i64 %305
  %319 = load double, ptr %318, align 8, !tbaa !6
  %320 = tail call double @llvm.fmuladd.f64(double %317, double %319, double %306)
  %321 = icmp eq i64 %313, %14
  br i1 %321, label %300, label %304, !llvm.loop !41

322:                                              ; preds = %300, %421
  %323 = phi i64 [ %325, %421 ], [ 0, %300 ]
  %324 = getelementptr inbounds double, ptr %12, i64 %323
  %325 = add nuw nsw i64 %323, 1
  %326 = trunc i64 %323 to i32
  %327 = trunc i64 %325 to i32
  %328 = add i32 %326, 1
  br i1 %24, label %418, label %329

329:                                              ; preds = %322
  %330 = insertelement <2 x i64> poison, i64 %323, i64 0
  %331 = shufflevector <2 x i64> %330, <2 x i64> poison, <2 x i32> zeroinitializer
  %332 = insertelement <2 x i32> poison, i32 %328, i64 0
  %333 = shufflevector <2 x i32> %332, <2 x i32> poison, <2 x i32> zeroinitializer
  %334 = insertelement <2 x i32> poison, i32 %327, i64 0
  %335 = shufflevector <2 x i32> %334, <2 x i32> poison, <2 x i32> zeroinitializer
  %336 = insertelement <2 x i32> poison, i32 %327, i64 0
  %337 = shufflevector <2 x i32> %336, <2 x i32> poison, <2 x i32> zeroinitializer
  %338 = insertelement <2 x i32> poison, i32 %327, i64 0
  %339 = shufflevector <2 x i32> %338, <2 x i32> poison, <2 x i32> zeroinitializer
  %340 = insertelement <2 x i32> poison, i32 %327, i64 0
  %341 = shufflevector <2 x i32> %340, <2 x i32> poison, <2 x i32> zeroinitializer
  %342 = add nuw i64 %323, 2
  %343 = insertelement <2 x i64> poison, i64 %342, i64 0
  %344 = shufflevector <2 x i64> %343, <2 x i64> poison, <2 x i32> zeroinitializer
  %345 = add nuw i64 %323, 4
  %346 = insertelement <2 x i64> poison, i64 %345, i64 0
  %347 = shufflevector <2 x i64> %346, <2 x i64> poison, <2 x i32> zeroinitializer
  %348 = add nuw i64 %323, 6
  %349 = insertelement <2 x i64> poison, i64 %348, i64 0
  %350 = shufflevector <2 x i64> %349, <2 x i64> poison, <2 x i32> zeroinitializer
  %351 = add i32 %326, 3
  %352 = insertelement <2 x i32> poison, i32 %351, i64 0
  %353 = shufflevector <2 x i32> %352, <2 x i32> poison, <2 x i32> zeroinitializer
  %354 = add i32 %326, 5
  %355 = insertelement <2 x i32> poison, i32 %354, i64 0
  %356 = shufflevector <2 x i32> %355, <2 x i32> poison, <2 x i32> zeroinitializer
  %357 = add i32 %326, 7
  %358 = insertelement <2 x i32> poison, i32 %357, i64 0
  %359 = shufflevector <2 x i32> %358, <2 x i32> poison, <2 x i32> zeroinitializer
  br label %360

360:                                              ; preds = %360, %329
  %361 = phi i64 [ 0, %329 ], [ %413, %360 ]
  %362 = phi <2 x i64> [ <i64 0, i64 1>, %329 ], [ %414, %360 ]
  %363 = phi double [ 0.000000e+00, %329 ], [ %412, %360 ]
  %364 = phi <2 x i32> [ <i32 0, i32 1>, %329 ], [ %415, %360 ]
  %365 = add nuw nsw <2 x i64> %362, %331
  %366 = add <2 x i64> %344, %362
  %367 = add <2 x i64> %347, %362
  %368 = add <2 x i64> %350, %362
  %369 = add <2 x i32> %333, %364
  %370 = add <2 x i32> %353, %364
  %371 = add <2 x i32> %356, %364
  %372 = add <2 x i32> %359, %364
  %373 = trunc <2 x i64> %365 to <2 x i32>
  %374 = trunc <2 x i64> %366 to <2 x i32>
  %375 = trunc <2 x i64> %367 to <2 x i32>
  %376 = trunc <2 x i64> %368 to <2 x i32>
  %377 = mul nsw <2 x i32> %369, %373
  %378 = mul nsw <2 x i32> %370, %374
  %379 = mul nsw <2 x i32> %371, %375
  %380 = mul nsw <2 x i32> %372, %376
  %381 = lshr <2 x i32> %377, <i32 1, i32 1>
  %382 = lshr <2 x i32> %378, <i32 1, i32 1>
  %383 = lshr <2 x i32> %379, <i32 1, i32 1>
  %384 = lshr <2 x i32> %380, <i32 1, i32 1>
  %385 = add <2 x i32> %381, %335
  %386 = add <2 x i32> %382, %337
  %387 = add <2 x i32> %383, %339
  %388 = add <2 x i32> %384, %341
  %389 = sitofp <2 x i32> %385 to <2 x double>
  %390 = sitofp <2 x i32> %386 to <2 x double>
  %391 = sitofp <2 x i32> %387 to <2 x double>
  %392 = sitofp <2 x i32> %388 to <2 x double>
  %393 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %389
  %394 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %390
  %395 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %391
  %396 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %392
  %397 = getelementptr inbounds double, ptr %11, i64 %361
  %398 = load <2 x double>, ptr %397, align 8, !tbaa !6
  %399 = getelementptr inbounds double, ptr %397, i64 2
  %400 = load <2 x double>, ptr %399, align 8, !tbaa !6
  %401 = getelementptr inbounds double, ptr %397, i64 4
  %402 = load <2 x double>, ptr %401, align 8, !tbaa !6
  %403 = getelementptr inbounds double, ptr %397, i64 6
  %404 = load <2 x double>, ptr %403, align 8, !tbaa !6
  %405 = fmul <2 x double> %393, %398
  %406 = fmul <2 x double> %394, %400
  %407 = fmul <2 x double> %395, %402
  %408 = fmul <2 x double> %396, %404
  %409 = tail call double @llvm.vector.reduce.fadd.v2f64(double %363, <2 x double> %405)
  %410 = tail call double @llvm.vector.reduce.fadd.v2f64(double %409, <2 x double> %406)
  %411 = tail call double @llvm.vector.reduce.fadd.v2f64(double %410, <2 x double> %407)
  %412 = tail call double @llvm.vector.reduce.fadd.v2f64(double %411, <2 x double> %408)
  %413 = add nuw i64 %361, 8
  %414 = add <2 x i64> %362, <i64 8, i64 8>
  %415 = add <2 x i32> %364, <i32 8, i32 8>
  %416 = icmp eq i64 %413, %25
  br i1 %416, label %417, label %360, !llvm.loop !42

417:                                              ; preds = %360
  br i1 %26, label %421, label %418

418:                                              ; preds = %322, %417
  %419 = phi i64 [ 0, %322 ], [ %25, %417 ]
  %420 = phi double [ 0.000000e+00, %322 ], [ %412, %417 ]
  br label %424

421:                                              ; preds = %424, %417
  %422 = phi double [ %412, %417 ], [ %438, %424 ]
  store double %422, ptr %324, align 8, !tbaa !6
  %423 = icmp eq i64 %325, %14
  br i1 %423, label %441, label %322, !llvm.loop !17

424:                                              ; preds = %418, %424
  %425 = phi i64 [ %439, %424 ], [ %419, %418 ]
  %426 = phi double [ %438, %424 ], [ %420, %418 ]
  %427 = trunc i64 %425 to i32
  %428 = add nuw nsw i64 %425, %323
  %429 = add i32 %328, %427
  %430 = trunc i64 %428 to i32
  %431 = mul nsw i32 %429, %430
  %432 = lshr i32 %431, 1
  %433 = add i32 %432, %327
  %434 = sitofp i32 %433 to double
  %435 = fdiv double 1.000000e+00, %434
  %436 = getelementptr inbounds double, ptr %11, i64 %425
  %437 = load double, ptr %436, align 8, !tbaa !6
  %438 = tail call double @llvm.fmuladd.f64(double %435, double %437, double %426)
  %439 = add nuw nsw i64 %425, 1
  %440 = icmp eq i64 %439, %14
  br i1 %440, label %421, label %424, !llvm.loop !43

441:                                              ; preds = %421, %538
  %442 = phi i64 [ %540, %538 ], [ 0, %421 ]
  %443 = getelementptr inbounds double, ptr %10, i64 %442
  %444 = trunc i64 %442 to i32
  %445 = add i32 %444, 1
  br i1 %27, label %535, label %446

446:                                              ; preds = %441
  %447 = insertelement <2 x i64> poison, i64 %442, i64 0
  %448 = shufflevector <2 x i64> %447, <2 x i64> poison, <2 x i32> zeroinitializer
  %449 = insertelement <2 x i32> poison, i32 %445, i64 0
  %450 = shufflevector <2 x i32> %449, <2 x i32> poison, <2 x i32> zeroinitializer
  %451 = add nuw i64 %442, 2
  %452 = insertelement <2 x i64> poison, i64 %451, i64 0
  %453 = shufflevector <2 x i64> %452, <2 x i64> poison, <2 x i32> zeroinitializer
  %454 = add nuw i64 %442, 4
  %455 = insertelement <2 x i64> poison, i64 %454, i64 0
  %456 = shufflevector <2 x i64> %455, <2 x i64> poison, <2 x i32> zeroinitializer
  %457 = add nuw i64 %442, 6
  %458 = insertelement <2 x i64> poison, i64 %457, i64 0
  %459 = shufflevector <2 x i64> %458, <2 x i64> poison, <2 x i32> zeroinitializer
  %460 = add i32 %444, 3
  %461 = insertelement <2 x i32> poison, i32 %460, i64 0
  %462 = shufflevector <2 x i32> %461, <2 x i32> poison, <2 x i32> zeroinitializer
  %463 = add i32 %444, 5
  %464 = insertelement <2 x i32> poison, i32 %463, i64 0
  %465 = shufflevector <2 x i32> %464, <2 x i32> poison, <2 x i32> zeroinitializer
  %466 = add i32 %444, 7
  %467 = insertelement <2 x i32> poison, i32 %466, i64 0
  %468 = shufflevector <2 x i32> %467, <2 x i32> poison, <2 x i32> zeroinitializer
  br label %469

469:                                              ; preds = %469, %446
  %470 = phi i64 [ 0, %446 ], [ %530, %469 ]
  %471 = phi <2 x i64> [ <i64 0, i64 1>, %446 ], [ %531, %469 ]
  %472 = phi double [ 0.000000e+00, %446 ], [ %529, %469 ]
  %473 = phi <2 x i32> [ <i32 0, i32 1>, %446 ], [ %532, %469 ]
  %474 = add nuw nsw <2 x i64> %471, %448
  %475 = add <2 x i64> %453, %471
  %476 = add <2 x i64> %456, %471
  %477 = add <2 x i64> %459, %471
  %478 = add <2 x i32> %450, %473
  %479 = add <2 x i32> %462, %473
  %480 = add <2 x i32> %465, %473
  %481 = add <2 x i32> %468, %473
  %482 = trunc <2 x i64> %474 to <2 x i32>
  %483 = trunc <2 x i64> %475 to <2 x i32>
  %484 = trunc <2 x i64> %476 to <2 x i32>
  %485 = trunc <2 x i64> %477 to <2 x i32>
  %486 = mul nsw <2 x i32> %478, %482
  %487 = mul nsw <2 x i32> %479, %483
  %488 = mul nsw <2 x i32> %480, %484
  %489 = mul nsw <2 x i32> %481, %485
  %490 = lshr <2 x i32> %486, <i32 1, i32 1>
  %491 = lshr <2 x i32> %487, <i32 1, i32 1>
  %492 = lshr <2 x i32> %488, <i32 1, i32 1>
  %493 = lshr <2 x i32> %489, <i32 1, i32 1>
  %494 = trunc <2 x i64> %471 to <2 x i32>
  %495 = add <2 x i32> %494, <i32 1, i32 1>
  %496 = trunc <2 x i64> %471 to <2 x i32>
  %497 = add <2 x i32> %496, <i32 3, i32 3>
  %498 = trunc <2 x i64> %471 to <2 x i32>
  %499 = add <2 x i32> %498, <i32 5, i32 5>
  %500 = trunc <2 x i64> %471 to <2 x i32>
  %501 = add <2 x i32> %500, <i32 7, i32 7>
  %502 = add nuw <2 x i32> %490, %495
  %503 = add nuw <2 x i32> %491, %497
  %504 = add nuw <2 x i32> %492, %499
  %505 = add nuw <2 x i32> %493, %501
  %506 = sitofp <2 x i32> %502 to <2 x double>
  %507 = sitofp <2 x i32> %503 to <2 x double>
  %508 = sitofp <2 x i32> %504 to <2 x double>
  %509 = sitofp <2 x i32> %505 to <2 x double>
  %510 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %506
  %511 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %507
  %512 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %508
  %513 = fdiv <2 x double> <double 1.000000e+00, double 1.000000e+00>, %509
  %514 = getelementptr inbounds double, ptr %12, i64 %470
  %515 = load <2 x double>, ptr %514, align 8, !tbaa !6
  %516 = getelementptr inbounds double, ptr %514, i64 2
  %517 = load <2 x double>, ptr %516, align 8, !tbaa !6
  %518 = getelementptr inbounds double, ptr %514, i64 4
  %519 = load <2 x double>, ptr %518, align 8, !tbaa !6
  %520 = getelementptr inbounds double, ptr %514, i64 6
  %521 = load <2 x double>, ptr %520, align 8, !tbaa !6
  %522 = fmul <2 x double> %510, %515
  %523 = fmul <2 x double> %511, %517
  %524 = fmul <2 x double> %512, %519
  %525 = fmul <2 x double> %513, %521
  %526 = tail call double @llvm.vector.reduce.fadd.v2f64(double %472, <2 x double> %522)
  %527 = tail call double @llvm.vector.reduce.fadd.v2f64(double %526, <2 x double> %523)
  %528 = tail call double @llvm.vector.reduce.fadd.v2f64(double %527, <2 x double> %524)
  %529 = tail call double @llvm.vector.reduce.fadd.v2f64(double %528, <2 x double> %525)
  %530 = add nuw i64 %470, 8
  %531 = add <2 x i64> %471, <i64 8, i64 8>
  %532 = add <2 x i32> %473, <i32 8, i32 8>
  %533 = icmp eq i64 %530, %28
  br i1 %533, label %534, label %469, !llvm.loop !44

534:                                              ; preds = %469
  br i1 %29, label %538, label %535

535:                                              ; preds = %441, %534
  %536 = phi i64 [ 0, %441 ], [ %28, %534 ]
  %537 = phi double [ 0.000000e+00, %441 ], [ %529, %534 ]
  br label %542

538:                                              ; preds = %542, %534
  %539 = phi double [ %529, %534 ], [ %558, %542 ]
  store double %539, ptr %443, align 8, !tbaa !6
  %540 = add nuw nsw i64 %442, 1
  %541 = icmp eq i64 %540, %14
  br i1 %541, label %560, label %441, !llvm.loop !23

542:                                              ; preds = %535, %542
  %543 = phi i64 [ %551, %542 ], [ %536, %535 ]
  %544 = phi double [ %558, %542 ], [ %537, %535 ]
  %545 = trunc i64 %543 to i32
  %546 = add nuw nsw i64 %543, %442
  %547 = add i32 %445, %545
  %548 = trunc i64 %546 to i32
  %549 = mul nsw i32 %547, %548
  %550 = lshr i32 %549, 1
  %551 = add nuw nsw i64 %543, 1
  %552 = trunc i64 %551 to i32
  %553 = add nuw i32 %550, %552
  %554 = sitofp i32 %553 to double
  %555 = fdiv double 1.000000e+00, %554
  %556 = getelementptr inbounds double, ptr %12, i64 %543
  %557 = load double, ptr %556, align 8, !tbaa !6
  %558 = tail call double @llvm.fmuladd.f64(double %555, double %557, double %544)
  %559 = icmp eq i64 %551, %14
  br i1 %559, label %538, label %542, !llvm.loop !45

560:                                              ; preds = %538
  %561 = add nuw nsw i32 %83, 1
  %562 = icmp eq i32 %561, 10
  br i1 %562, label %30, label %563

563:                                              ; preds = %560, %565
  %564 = phi i32 [ %561, %560 ], [ %566, %565 ]
  br label %82, !llvm.loop !46

565:                                              ; preds = %82
  %566 = add nuw nsw i32 %83, 1
  %567 = icmp eq i32 %566, 10
  br i1 %567, label %572, label %563

568:                                              ; preds = %576, %76
  %569 = phi double [ %73, %76 ], [ %584, %576 ]
  %570 = phi double [ %69, %76 ], [ %585, %576 ]
  %571 = fdiv double %569, %570
  br label %572

572:                                              ; preds = %565, %568, %30
  %573 = phi double [ %571, %568 ], [ 0x7FF8000000000000, %30 ], [ 0x7FF8000000000000, %565 ]
  %574 = tail call double @llvm.sqrt.f64(double %573)
  %575 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, double noundef %574)
  tail call void @free(ptr noundef %10)
  tail call void @free(ptr noundef %11)
  tail call void @free(ptr noundef %12)
  br label %588

576:                                              ; preds = %78, %576
  %577 = phi i64 [ %586, %576 ], [ %79, %78 ]
  %578 = phi double [ %585, %576 ], [ %80, %78 ]
  %579 = phi double [ %584, %576 ], [ %81, %78 ]
  %580 = getelementptr inbounds double, ptr %10, i64 %577
  %581 = load double, ptr %580, align 8, !tbaa !6
  %582 = getelementptr inbounds double, ptr %11, i64 %577
  %583 = load double, ptr %582, align 8, !tbaa !6
  %584 = tail call double @llvm.fmuladd.f64(double %581, double %583, double %579)
  %585 = tail call double @llvm.fmuladd.f64(double %583, double %583, double %578)
  %586 = add nuw nsw i64 %577, 1
  %587 = icmp eq i64 %586, %14
  br i1 %587, label %568, label %576, !llvm.loop !47

588:                                              ; preds = %2, %572
  %589 = phi i32 [ 0, %572 ], [ 1, %2 ]
  ret i32 %589
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i32 @atoi(ptr nocapture noundef) local_unnamed_addr #4

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #5

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #6

; Function Attrs: mustprogress nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.sqrt.f64(double) #2

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #7

; Function Attrs: nofree nounwind willreturn memory(argmem: readwrite)
declare void @memset_pattern16(ptr nocapture writeonly, ptr nocapture readonly, i64) local_unnamed_addr #8

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.vector.reduce.fadd.v2f64(double, <2 x double>) #9

attributes #0 = { mustprogress nofree norecurse nosync nounwind ssp willreturn memory(none) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { nofree nosync nounwind ssp memory(argmem: readwrite) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #3 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #6 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #7 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #8 = { nofree nounwind willreturn memory(argmem: readwrite) }
attributes #9 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #10 = { allocsize(0) }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 2, !"SDK Version", [2 x i32] [i32 15, i32 2]}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"uwtable", i32 1}
!4 = !{i32 7, !"frame-pointer", i32 1}
!5 = !{!"Apple clang version 16.0.0 (clang-1600.0.26.6)"}
!6 = !{!7, !7, i64 0}
!7 = !{!"double", !8, i64 0}
!8 = !{!"omnipotent char", !9, i64 0}
!9 = !{!"Simple C/C++ TBAA"}
!10 = !{!11}
!11 = distinct !{!11, !12}
!12 = distinct !{!12, !"LVerDomain"}
!13 = distinct !{!13, !14, !15, !16}
!14 = !{!"llvm.loop.mustprogress"}
!15 = !{!"llvm.loop.isvectorized", i32 1}
!16 = !{!"llvm.loop.unroll.runtime.disable"}
!17 = distinct !{!17, !14}
!18 = distinct !{!18, !14, !15}
!19 = !{!20}
!20 = distinct !{!20, !21}
!21 = distinct !{!21, !"LVerDomain"}
!22 = distinct !{!22, !14, !15, !16}
!23 = distinct !{!23, !14}
!24 = distinct !{!24, !14, !15}
!25 = !{!26}
!26 = distinct !{!26, !27}
!27 = distinct !{!27, !"LVerDomain"}
!28 = distinct !{!28, !14, !15, !16}
!29 = distinct !{!29, !14, !15}
!30 = !{!31}
!31 = distinct !{!31, !32}
!32 = distinct !{!32, !"LVerDomain"}
!33 = distinct !{!33, !14, !15, !16}
!34 = distinct !{!34, !14, !15}
!35 = !{!36, !36, i64 0}
!36 = !{!"any pointer", !8, i64 0}
!37 = distinct !{!37, !14, !15, !16}
!38 = distinct !{!38, !14, !15, !16}
!39 = distinct !{!39, !14, !16, !15}
!40 = distinct !{!40, !14, !15, !16}
!41 = distinct !{!41, !14, !16, !15}
!42 = distinct !{!42, !14, !15, !16}
!43 = distinct !{!43, !14, !16, !15}
!44 = distinct !{!44, !14, !15, !16}
!45 = distinct !{!45, !14, !16, !15}
!46 = distinct !{!46, !14}
!47 = distinct !{!47, !14, !15}
