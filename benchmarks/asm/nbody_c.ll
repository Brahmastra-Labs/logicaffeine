; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/nbody/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/nbody/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

%struct.Body = type { double, double, double, double, double, double, double }

@bodies = local_unnamed_addr global [5 x %struct.Body] [%struct.Body { double 0.000000e+00, double 0.000000e+00, double 0.000000e+00, double 0.000000e+00, double 0.000000e+00, double 0.000000e+00, double 0x4043BD3CC9BE45DE }, %struct.Body { double 0x40135DA0343CD92C, double 0xBFF290ABC01FDB7C, double 0xBFBA86F96C25EBF0, double 0x3FE367069B93CCBC, double 0x40067EF2F57D949B, double 0xBF99D2D79A5A0715, double 0x3FA34C95D9AB33D8 }, %struct.Body { double 0x4020AFCDC332CA67, double 0x40107FCB31DE01B0, double 0xBFD9D353E1EB467C, double 0xBFF02C21B8879442, double 0x3FFD35E9BF1F8F13, double 0x3F813C485F1123B4, double 0x3F871D490D07C637 }, %struct.Body { double 0x4029C9EACEA7D9CF, double 0xC02E38E8D626667E, double 0xBFCC9557BE257DA0, double 0x3FF1531CA9911BEF, double 0x3FEBCC7F3E54BBC5, double 0xBF862F6BFAF23E7C, double 0x3F5C3DD29CF41EB3 }, %struct.Body { double 0x402EC267A905572A, double 0xC039EB5833C8A220, double 0x3FC6F1F393ABE540, double 0x3FEF54B61659BC4A, double 0x3FE307C631C4FBA3, double 0xBFA1CB88587665F6, double 0x3F60A8F3531799AC }], align 16
@.str = private unnamed_addr constant [6 x i8] c"%.9f\0A\00", align 1

; Function Attrs: mustprogress nofree nosync nounwind ssp willreturn memory(readwrite, argmem: none, inaccessiblemem: none) uwtable(sync)
define void @offset_momentum() local_unnamed_addr #0 {
  %1 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 6), align 8, !tbaa !6
  %2 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 5), align 8, !tbaa !11
  %3 = tail call double @llvm.fmuladd.f64(double %2, double %1, double 0.000000e+00)
  %4 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 6), align 8, !tbaa !6
  %5 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 5), align 8, !tbaa !11
  %6 = tail call double @llvm.fmuladd.f64(double %5, double %4, double %3)
  %7 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 6), align 8, !tbaa !6
  %8 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 5), align 8, !tbaa !11
  %9 = tail call double @llvm.fmuladd.f64(double %8, double %7, double %6)
  %10 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 6), align 8, !tbaa !6
  %11 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 5), align 8, !tbaa !11
  %12 = tail call double @llvm.fmuladd.f64(double %11, double %10, double %9)
  %13 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 6), align 8, !tbaa !6
  %14 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 5), align 8, !tbaa !11
  %15 = tail call double @llvm.fmuladd.f64(double %14, double %13, double %12)
  %16 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 3), align 8, !tbaa !12
  %17 = insertelement <2 x double> poison, double %1, i64 0
  %18 = shufflevector <2 x double> %17, <2 x double> poison, <2 x i32> zeroinitializer
  %19 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %16, <2 x double> %18, <2 x double> zeroinitializer)
  %20 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 3), align 8, !tbaa !12
  %21 = insertelement <2 x double> poison, double %4, i64 0
  %22 = shufflevector <2 x double> %21, <2 x double> poison, <2 x i32> zeroinitializer
  %23 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %20, <2 x double> %22, <2 x double> %19)
  %24 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 3), align 8, !tbaa !12
  %25 = insertelement <2 x double> poison, double %7, i64 0
  %26 = shufflevector <2 x double> %25, <2 x double> poison, <2 x i32> zeroinitializer
  %27 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %24, <2 x double> %26, <2 x double> %23)
  %28 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 3), align 8, !tbaa !12
  %29 = insertelement <2 x double> poison, double %10, i64 0
  %30 = shufflevector <2 x double> %29, <2 x double> poison, <2 x i32> zeroinitializer
  %31 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %28, <2 x double> %30, <2 x double> %27)
  %32 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 3), align 8, !tbaa !12
  %33 = insertelement <2 x double> poison, double %13, i64 0
  %34 = shufflevector <2 x double> %33, <2 x double> poison, <2 x i32> zeroinitializer
  %35 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %32, <2 x double> %34, <2 x double> %31)
  %36 = fdiv <2 x double> %35, <double 0xC043BD3CC9BE45DE, double 0xC043BD3CC9BE45DE>
  store <2 x double> %36, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 3), align 8, !tbaa !12
  %37 = fdiv double %15, 0xC043BD3CC9BE45DE
  store double %37, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 5), align 8, !tbaa !11
  ret void
}

; Function Attrs: mustprogress nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.fmuladd.f64(double, double, double) #1

; Function Attrs: nofree nosync nounwind ssp memory(read, argmem: none, inaccessiblemem: none) uwtable(sync)
define double @energy() local_unnamed_addr #2 {
  br label %6

1:                                                ; preds = %32, %6
  %2 = phi double [ %22, %6 ], [ %52, %32 ]
  %3 = add nuw nsw i64 %8, 1
  %4 = icmp eq i64 %23, 5
  br i1 %4, label %5, label %6, !llvm.loop !13

5:                                                ; preds = %1
  ret double %2

6:                                                ; preds = %0, %1
  %7 = phi i64 [ 0, %0 ], [ %23, %1 ]
  %8 = phi i64 [ 1, %0 ], [ %3, %1 ]
  %9 = phi double [ 0.000000e+00, %0 ], [ %2, %1 ]
  %10 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %7, i32 6
  %11 = load double, ptr %10, align 8, !tbaa !6
  %12 = fmul double %11, 5.000000e-01
  %13 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %7, i32 3
  %14 = load double, ptr %13, align 8, !tbaa !15
  %15 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %7, i32 4
  %16 = load double, ptr %15, align 8, !tbaa !16
  %17 = fmul double %16, %16
  %18 = tail call double @llvm.fmuladd.f64(double %14, double %14, double %17)
  %19 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %7, i32 5
  %20 = load double, ptr %19, align 8, !tbaa !11
  %21 = tail call double @llvm.fmuladd.f64(double %20, double %20, double %18)
  %22 = tail call double @llvm.fmuladd.f64(double %12, double %21, double %9)
  %23 = add nuw nsw i64 %7, 1
  %24 = icmp ult i64 %7, 4
  br i1 %24, label %25, label %1

25:                                               ; preds = %6
  %26 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %7
  %27 = load double, ptr %26, align 8, !tbaa !17
  %28 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %7, i32 1
  %29 = load double, ptr %28, align 8, !tbaa !18
  %30 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %7, i32 2
  %31 = load double, ptr %30, align 8, !tbaa !19
  br label %32

32:                                               ; preds = %25, %32
  %33 = phi i64 [ %8, %25 ], [ %53, %32 ]
  %34 = phi double [ %22, %25 ], [ %52, %32 ]
  %35 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %33
  %36 = load double, ptr %35, align 8, !tbaa !17
  %37 = fsub double %27, %36
  %38 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %33, i32 1
  %39 = load double, ptr %38, align 8, !tbaa !18
  %40 = fsub double %29, %39
  %41 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %33, i32 2
  %42 = load double, ptr %41, align 8, !tbaa !19
  %43 = fsub double %31, %42
  %44 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %33, i32 6
  %45 = load double, ptr %44, align 8, !tbaa !6
  %46 = fmul double %11, %45
  %47 = fmul double %40, %40
  %48 = tail call double @llvm.fmuladd.f64(double %37, double %37, double %47)
  %49 = tail call double @llvm.fmuladd.f64(double %43, double %43, double %48)
  %50 = tail call double @llvm.sqrt.f64(double %49)
  %51 = fdiv double %46, %50
  %52 = fsub double %34, %51
  %53 = add nuw nsw i64 %33, 1
  %54 = icmp eq i64 %53, 5
  br i1 %54, label %1, label %32, !llvm.loop !20
}

; Function Attrs: mustprogress nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare double @llvm.sqrt.f64(double) #1

; Function Attrs: nofree nosync nounwind ssp memory(readwrite, argmem: none, inaccessiblemem: none) uwtable(sync)
define void @advance(double noundef %0) local_unnamed_addr #3 {
  br label %38

2:                                                ; preds = %54, %38
  %3 = add nuw nsw i64 %40, 1
  %4 = icmp eq i64 %41, 5
  br i1 %4, label %5, label %38, !llvm.loop !21

5:                                                ; preds = %2
  %6 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 3), align 8, !tbaa !12
  %7 = load <2 x double>, ptr @bodies, align 16, !tbaa !12
  %8 = insertelement <2 x double> poison, double %0, i64 0
  %9 = shufflevector <2 x double> %8, <2 x double> poison, <2 x i32> zeroinitializer
  %10 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %9, <2 x double> %6, <2 x double> %7)
  store <2 x double> %10, ptr @bodies, align 16, !tbaa !12
  %11 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 5), align 8, !tbaa !11
  %12 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 2), align 16, !tbaa !19
  %13 = tail call double @llvm.fmuladd.f64(double %0, double %11, double %12)
  store double %13, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 2), align 16, !tbaa !19
  %14 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 3), align 16, !tbaa !12
  %15 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1), align 8, !tbaa !12
  %16 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %9, <2 x double> %14, <2 x double> %15)
  store <2 x double> %16, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1), align 8, !tbaa !12
  %17 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 5), align 16, !tbaa !11
  %18 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 2), align 8, !tbaa !19
  %19 = tail call double @llvm.fmuladd.f64(double %0, double %17, double %18)
  store double %19, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 2), align 8, !tbaa !19
  %20 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 3), align 8, !tbaa !12
  %21 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2), align 16, !tbaa !12
  %22 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %9, <2 x double> %20, <2 x double> %21)
  store <2 x double> %22, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2), align 16, !tbaa !12
  %23 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 5), align 8, !tbaa !11
  %24 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 2), align 16, !tbaa !19
  %25 = tail call double @llvm.fmuladd.f64(double %0, double %23, double %24)
  store double %25, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 2), align 16, !tbaa !19
  %26 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 3), align 16, !tbaa !12
  %27 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3), align 8, !tbaa !12
  %28 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %9, <2 x double> %26, <2 x double> %27)
  store <2 x double> %28, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3), align 8, !tbaa !12
  %29 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 5), align 16, !tbaa !11
  %30 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 2), align 8, !tbaa !19
  %31 = tail call double @llvm.fmuladd.f64(double %0, double %29, double %30)
  store double %31, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 2), align 8, !tbaa !19
  %32 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 3), align 8, !tbaa !12
  %33 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4), align 16, !tbaa !12
  %34 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %9, <2 x double> %32, <2 x double> %33)
  store <2 x double> %34, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4), align 16, !tbaa !12
  %35 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 5), align 8, !tbaa !11
  %36 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 2), align 16, !tbaa !19
  %37 = tail call double @llvm.fmuladd.f64(double %0, double %35, double %36)
  store double %37, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 2), align 16, !tbaa !19
  ret void

38:                                               ; preds = %1, %2
  %39 = phi i64 [ 0, %1 ], [ %41, %2 ]
  %40 = phi i64 [ 1, %1 ], [ %3, %2 ]
  %41 = add nuw nsw i64 %39, 1
  %42 = icmp ult i64 %39, 4
  br i1 %42, label %43, label %2

43:                                               ; preds = %38
  %44 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %39
  %45 = load <2 x double>, ptr %44, align 8, !tbaa !12
  %46 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %39, i32 2
  %47 = load double, ptr %46, align 8, !tbaa !19
  %48 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %39, i32 3
  %49 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %39, i32 5
  %50 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %39, i32 6
  %51 = load double, ptr %50, align 8, !tbaa !6
  %52 = insertelement <2 x double> poison, double %51, i64 0
  %53 = shufflevector <2 x double> %52, <2 x double> poison, <2 x i32> zeroinitializer
  br label %54

54:                                               ; preds = %43, %54
  %55 = phi i64 [ %40, %43 ], [ %93, %54 ]
  %56 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %55
  %57 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %55, i32 2
  %58 = load double, ptr %57, align 8, !tbaa !19
  %59 = fsub double %47, %58
  %60 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %55, i32 6
  %61 = load double, ptr %60, align 8, !tbaa !6
  %62 = load double, ptr %49, align 8, !tbaa !11
  %63 = fneg double %59
  %64 = fmul double %61, %63
  %65 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %55, i32 3
  %66 = load <2 x double>, ptr %56, align 8, !tbaa !12
  %67 = fsub <2 x double> %45, %66
  %68 = fmul <2 x double> %67, %67
  %69 = extractelement <2 x double> %68, i64 1
  %70 = extractelement <2 x double> %67, i64 0
  %71 = tail call double @llvm.fmuladd.f64(double %70, double %70, double %69)
  %72 = tail call double @llvm.fmuladd.f64(double %59, double %59, double %71)
  %73 = tail call double @llvm.sqrt.f64(double %72)
  %74 = fmul double %73, %73
  %75 = fmul double %73, %74
  %76 = fdiv double %0, %75
  %77 = load <2 x double>, ptr %48, align 8, !tbaa !12
  %78 = fneg <2 x double> %67
  %79 = insertelement <2 x double> poison, double %61, i64 0
  %80 = shufflevector <2 x double> %79, <2 x double> poison, <2 x i32> zeroinitializer
  %81 = fmul <2 x double> %80, %78
  %82 = insertelement <2 x double> poison, double %76, i64 0
  %83 = shufflevector <2 x double> %82, <2 x double> poison, <2 x i32> zeroinitializer
  %84 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %81, <2 x double> %83, <2 x double> %77)
  store <2 x double> %84, ptr %48, align 8, !tbaa !12
  %85 = tail call double @llvm.fmuladd.f64(double %64, double %76, double %62)
  store double %85, ptr %49, align 8, !tbaa !11
  %86 = fmul <2 x double> %67, %53
  %87 = load <2 x double>, ptr %65, align 8, !tbaa !12
  %88 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %86, <2 x double> %83, <2 x double> %87)
  store <2 x double> %88, ptr %65, align 8, !tbaa !12
  %89 = fmul double %59, %51
  %90 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %55, i32 5
  %91 = load double, ptr %90, align 8, !tbaa !11
  %92 = tail call double @llvm.fmuladd.f64(double %89, double %76, double %91)
  store double %92, ptr %90, align 8, !tbaa !11
  %93 = add nuw nsw i64 %55, 1
  %94 = icmp eq i64 %93, 5
  br i1 %94, label %2, label %54, !llvm.loop !22
}

; Function Attrs: nofree nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #4 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %161, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !23
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 6), align 16, !tbaa !6
  %9 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 5), align 8, !tbaa !11
  %10 = tail call double @llvm.fmuladd.f64(double %9, double %8, double 0.000000e+00)
  %11 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 6), align 8, !tbaa !6
  %12 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 5), align 16, !tbaa !11
  %13 = tail call double @llvm.fmuladd.f64(double %12, double %11, double %10)
  %14 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 6), align 16, !tbaa !6
  %15 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 5), align 8, !tbaa !11
  %16 = tail call double @llvm.fmuladd.f64(double %15, double %14, double %13)
  %17 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 6), align 8, !tbaa !6
  %18 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 5), align 16, !tbaa !11
  %19 = tail call double @llvm.fmuladd.f64(double %18, double %17, double %16)
  %20 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 6), align 16, !tbaa !6
  %21 = load double, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 5), align 8, !tbaa !11
  %22 = tail call double @llvm.fmuladd.f64(double %21, double %20, double %19)
  %23 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 3), align 8, !tbaa !12
  %24 = insertelement <2 x double> poison, double %8, i64 0
  %25 = shufflevector <2 x double> %24, <2 x double> poison, <2 x i32> zeroinitializer
  %26 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %23, <2 x double> %25, <2 x double> zeroinitializer)
  %27 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 1, i32 3), align 16, !tbaa !12
  %28 = insertelement <2 x double> poison, double %11, i64 0
  %29 = shufflevector <2 x double> %28, <2 x double> poison, <2 x i32> zeroinitializer
  %30 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %27, <2 x double> %29, <2 x double> %26)
  %31 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 2, i32 3), align 8, !tbaa !12
  %32 = insertelement <2 x double> poison, double %14, i64 0
  %33 = shufflevector <2 x double> %32, <2 x double> poison, <2 x i32> zeroinitializer
  %34 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %31, <2 x double> %33, <2 x double> %30)
  %35 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 3, i32 3), align 16, !tbaa !12
  %36 = insertelement <2 x double> poison, double %17, i64 0
  %37 = shufflevector <2 x double> %36, <2 x double> poison, <2 x i32> zeroinitializer
  %38 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %35, <2 x double> %37, <2 x double> %34)
  %39 = load <2 x double>, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 4, i32 3), align 8, !tbaa !12
  %40 = insertelement <2 x double> poison, double %20, i64 0
  %41 = shufflevector <2 x double> %40, <2 x double> poison, <2 x i32> zeroinitializer
  %42 = tail call <2 x double> @llvm.fmuladd.v2f64(<2 x double> %39, <2 x double> %41, <2 x double> %38)
  %43 = fdiv <2 x double> %42, <double 0xC043BD3CC9BE45DE, double 0xC043BD3CC9BE45DE>
  store <2 x double> %43, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 3), align 8, !tbaa !12
  %44 = fdiv double %22, 0xC043BD3CC9BE45DE
  store double %44, ptr getelementptr inbounds ([5 x %struct.Body], ptr @bodies, i64 0, i64 0, i32 5), align 8, !tbaa !11
  br label %49

45:                                               ; preds = %75, %49
  %46 = phi double [ %65, %49 ], [ %95, %75 ]
  %47 = add nuw nsw i64 %51, 1
  %48 = icmp eq i64 %66, 5
  br i1 %48, label %98, label %49, !llvm.loop !13

49:                                               ; preds = %45, %4
  %50 = phi i64 [ 0, %4 ], [ %66, %45 ]
  %51 = phi i64 [ 1, %4 ], [ %47, %45 ]
  %52 = phi double [ 0.000000e+00, %4 ], [ %46, %45 ]
  %53 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %50, i32 6
  %54 = load double, ptr %53, align 8, !tbaa !6
  %55 = fmul double %54, 5.000000e-01
  %56 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %50, i32 3
  %57 = load double, ptr %56, align 8, !tbaa !15
  %58 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %50, i32 4
  %59 = load double, ptr %58, align 8, !tbaa !16
  %60 = fmul double %59, %59
  %61 = tail call double @llvm.fmuladd.f64(double %57, double %57, double %60)
  %62 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %50, i32 5
  %63 = load double, ptr %62, align 8, !tbaa !11
  %64 = tail call double @llvm.fmuladd.f64(double %63, double %63, double %61)
  %65 = tail call double @llvm.fmuladd.f64(double %55, double %64, double %52)
  %66 = add nuw nsw i64 %50, 1
  %67 = icmp ult i64 %50, 4
  br i1 %67, label %68, label %45

68:                                               ; preds = %49
  %69 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %50
  %70 = load double, ptr %69, align 8, !tbaa !17
  %71 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %50, i32 1
  %72 = load double, ptr %71, align 8, !tbaa !18
  %73 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %50, i32 2
  %74 = load double, ptr %73, align 8, !tbaa !19
  br label %75

75:                                               ; preds = %75, %68
  %76 = phi i64 [ %51, %68 ], [ %96, %75 ]
  %77 = phi double [ %65, %68 ], [ %95, %75 ]
  %78 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %76
  %79 = load double, ptr %78, align 8, !tbaa !17
  %80 = fsub double %70, %79
  %81 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %76, i32 1
  %82 = load double, ptr %81, align 8, !tbaa !18
  %83 = fsub double %72, %82
  %84 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %76, i32 2
  %85 = load double, ptr %84, align 8, !tbaa !19
  %86 = fsub double %74, %85
  %87 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %76, i32 6
  %88 = load double, ptr %87, align 8, !tbaa !6
  %89 = fmul double %54, %88
  %90 = fmul double %83, %83
  %91 = tail call double @llvm.fmuladd.f64(double %80, double %80, double %90)
  %92 = tail call double @llvm.fmuladd.f64(double %86, double %86, double %91)
  %93 = tail call double @llvm.sqrt.f64(double %92)
  %94 = fdiv double %89, %93
  %95 = fsub double %77, %94
  %96 = add nuw nsw i64 %76, 1
  %97 = icmp eq i64 %96, 5
  br i1 %97, label %45, label %75, !llvm.loop !20

98:                                               ; preds = %45
  %99 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, double noundef %46)
  %100 = icmp sgt i64 %7, 0
  br i1 %100, label %157, label %101

101:                                              ; preds = %157, %98
  br label %106

102:                                              ; preds = %132, %106
  %103 = phi double [ %122, %106 ], [ %152, %132 ]
  %104 = add nuw nsw i64 %108, 1
  %105 = icmp eq i64 %123, 5
  br i1 %105, label %155, label %106, !llvm.loop !13

106:                                              ; preds = %101, %102
  %107 = phi i64 [ %123, %102 ], [ 0, %101 ]
  %108 = phi i64 [ %104, %102 ], [ 1, %101 ]
  %109 = phi double [ %103, %102 ], [ 0.000000e+00, %101 ]
  %110 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %107, i32 6
  %111 = load double, ptr %110, align 8, !tbaa !6
  %112 = fmul double %111, 5.000000e-01
  %113 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %107, i32 3
  %114 = load double, ptr %113, align 8, !tbaa !15
  %115 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %107, i32 4
  %116 = load double, ptr %115, align 8, !tbaa !16
  %117 = fmul double %116, %116
  %118 = tail call double @llvm.fmuladd.f64(double %114, double %114, double %117)
  %119 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %107, i32 5
  %120 = load double, ptr %119, align 8, !tbaa !11
  %121 = tail call double @llvm.fmuladd.f64(double %120, double %120, double %118)
  %122 = tail call double @llvm.fmuladd.f64(double %112, double %121, double %109)
  %123 = add nuw nsw i64 %107, 1
  %124 = icmp ult i64 %107, 4
  br i1 %124, label %125, label %102

125:                                              ; preds = %106
  %126 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %107
  %127 = load double, ptr %126, align 8, !tbaa !17
  %128 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %107, i32 1
  %129 = load double, ptr %128, align 8, !tbaa !18
  %130 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %107, i32 2
  %131 = load double, ptr %130, align 8, !tbaa !19
  br label %132

132:                                              ; preds = %132, %125
  %133 = phi i64 [ %108, %125 ], [ %153, %132 ]
  %134 = phi double [ %122, %125 ], [ %152, %132 ]
  %135 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %133
  %136 = load double, ptr %135, align 8, !tbaa !17
  %137 = fsub double %127, %136
  %138 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %133, i32 1
  %139 = load double, ptr %138, align 8, !tbaa !18
  %140 = fsub double %129, %139
  %141 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %133, i32 2
  %142 = load double, ptr %141, align 8, !tbaa !19
  %143 = fsub double %131, %142
  %144 = getelementptr inbounds [5 x %struct.Body], ptr @bodies, i64 0, i64 %133, i32 6
  %145 = load double, ptr %144, align 8, !tbaa !6
  %146 = fmul double %111, %145
  %147 = fmul double %140, %140
  %148 = tail call double @llvm.fmuladd.f64(double %137, double %137, double %147)
  %149 = tail call double @llvm.fmuladd.f64(double %143, double %143, double %148)
  %150 = tail call double @llvm.sqrt.f64(double %149)
  %151 = fdiv double %146, %150
  %152 = fsub double %134, %151
  %153 = add nuw nsw i64 %133, 1
  %154 = icmp eq i64 %153, 5
  br i1 %154, label %102, label %132, !llvm.loop !20

155:                                              ; preds = %102
  %156 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, double noundef %103)
  br label %161

157:                                              ; preds = %98, %157
  %158 = phi i64 [ %159, %157 ], [ 0, %98 ]
  tail call void @advance(double noundef 1.000000e-02)
  %159 = add nuw nsw i64 %158, 1
  %160 = icmp eq i64 %159, %7
  br i1 %160, label %101, label %157, !llvm.loop !25

161:                                              ; preds = %2, %155
  %162 = phi i32 [ 0, %155 ], [ 1, %2 ]
  ret i32 %162
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #5

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #6

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare <2 x double> @llvm.fmuladd.v2f64(<2 x double>, <2 x double>, <2 x double>) #7

attributes #0 = { mustprogress nofree nosync nounwind ssp willreturn memory(readwrite, argmem: none, inaccessiblemem: none) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #2 = { nofree nosync nounwind ssp memory(read, argmem: none, inaccessiblemem: none) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nosync nounwind ssp memory(readwrite, argmem: none, inaccessiblemem: none) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { nofree nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #6 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #7 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 2, !"SDK Version", [2 x i32] [i32 15, i32 2]}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"uwtable", i32 1}
!4 = !{i32 7, !"frame-pointer", i32 1}
!5 = !{!"Apple clang version 16.0.0 (clang-1600.0.26.6)"}
!6 = !{!7, !8, i64 48}
!7 = !{!"", !8, i64 0, !8, i64 8, !8, i64 16, !8, i64 24, !8, i64 32, !8, i64 40, !8, i64 48}
!8 = !{!"double", !9, i64 0}
!9 = !{!"omnipotent char", !10, i64 0}
!10 = !{!"Simple C/C++ TBAA"}
!11 = !{!7, !8, i64 40}
!12 = !{!8, !8, i64 0}
!13 = distinct !{!13, !14}
!14 = !{!"llvm.loop.mustprogress"}
!15 = !{!7, !8, i64 24}
!16 = !{!7, !8, i64 32}
!17 = !{!7, !8, i64 0}
!18 = !{!7, !8, i64 8}
!19 = !{!7, !8, i64 16}
!20 = distinct !{!20, !14}
!21 = distinct !{!21, !14}
!22 = distinct !{!22, !14}
!23 = !{!24, !24, i64 0}
!24 = !{!"any pointer", !9, i64 0}
!25 = distinct !{!25, !14}
