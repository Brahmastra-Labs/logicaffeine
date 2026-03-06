; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/counting_sort/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/counting_sort/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [13 x i8] c"%ld %ld %ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = alloca [1000 x i64], align 8
  %4 = icmp slt i32 %0, 2
  br i1 %4, label %96, label %5

5:                                                ; preds = %2
  %6 = getelementptr inbounds ptr, ptr %1, i64 1
  %7 = load ptr, ptr %6, align 8, !tbaa !6
  %8 = tail call i64 @atol(ptr nocapture noundef %7)
  %9 = shl i64 %8, 3
  %10 = tail call ptr @malloc(i64 noundef %9) #7
  %11 = icmp sgt i64 %8, 0
  br i1 %11, label %14, label %12

12:                                               ; preds = %5
  call void @llvm.lifetime.start.p0(i64 8000, ptr nonnull %3) #8
  call void @llvm.memset.p0.i64(ptr noundef nonnull align 8 dereferenceable(8000) %3, i8 0, i64 8000, i1 false)
  br label %36

13:                                               ; preds = %14
  call void @llvm.lifetime.start.p0(i64 8000, ptr nonnull %3) #8
  call void @llvm.memset.p0.i64(ptr noundef nonnull align 8 dereferenceable(8000) %3, i8 0, i64 8000, i1 false)
  br i1 %11, label %27, label %36

14:                                               ; preds = %5, %14
  %15 = phi i64 [ %25, %14 ], [ 0, %5 ]
  %16 = phi i32 [ %19, %14 ], [ 42, %5 ]
  %17 = mul i32 %16, 1103515245
  %18 = add i32 %17, 12345
  %19 = and i32 %18, 2147483647
  %20 = lshr i32 %19, 16
  %21 = trunc i32 %20 to i16
  %22 = urem i16 %21, 1000
  %23 = zext i16 %22 to i64
  %24 = getelementptr inbounds i64, ptr %10, i64 %15
  store i64 %23, ptr %24, align 8, !tbaa !10
  %25 = add nuw nsw i64 %15, 1
  %26 = icmp eq i64 %25, %8
  br i1 %26, label %13, label %14, !llvm.loop !12

27:                                               ; preds = %13, %27
  %28 = phi i64 [ %34, %27 ], [ 0, %13 ]
  %29 = getelementptr inbounds i64, ptr %10, i64 %28
  %30 = load i64, ptr %29, align 8, !tbaa !10
  %31 = getelementptr inbounds [1000 x i64], ptr %3, i64 0, i64 %30
  %32 = load i64, ptr %31, align 8, !tbaa !10
  %33 = add nsw i64 %32, 1
  store i64 %33, ptr %31, align 8, !tbaa !10
  %34 = add nuw nsw i64 %28, 1
  %35 = icmp eq i64 %34, %8
  br i1 %35, label %36, label %27, !llvm.loop !14

36:                                               ; preds = %27, %12, %13
  br label %37

37:                                               ; preds = %36, %71
  %38 = phi i64 [ %73, %71 ], [ 0, %36 ]
  %39 = phi i64 [ %72, %71 ], [ 0, %36 ]
  %40 = getelementptr inbounds [1000 x i64], ptr %3, i64 0, i64 %38
  %41 = load i64, ptr %40, align 8, !tbaa !10
  %42 = icmp sgt i64 %41, 0
  br i1 %42, label %43, label %71

43:                                               ; preds = %37
  %44 = add i64 %39, %41
  %45 = icmp ult i64 %41, 8
  br i1 %45, label %68, label %46

46:                                               ; preds = %43
  %47 = and i64 %41, -8
  %48 = add i64 %39, %47
  %49 = insertelement <2 x i64> poison, i64 %38, i64 0
  %50 = shufflevector <2 x i64> %49, <2 x i64> poison, <2 x i32> zeroinitializer
  %51 = insertelement <2 x i64> poison, i64 %38, i64 0
  %52 = shufflevector <2 x i64> %51, <2 x i64> poison, <2 x i32> zeroinitializer
  %53 = insertelement <2 x i64> poison, i64 %38, i64 0
  %54 = shufflevector <2 x i64> %53, <2 x i64> poison, <2 x i32> zeroinitializer
  %55 = insertelement <2 x i64> poison, i64 %38, i64 0
  %56 = shufflevector <2 x i64> %55, <2 x i64> poison, <2 x i32> zeroinitializer
  br label %57

57:                                               ; preds = %57, %46
  %58 = phi i64 [ 0, %46 ], [ %64, %57 ]
  %59 = add i64 %39, %58
  %60 = getelementptr inbounds i64, ptr %10, i64 %59
  store <2 x i64> %50, ptr %60, align 8, !tbaa !10
  %61 = getelementptr inbounds i64, ptr %60, i64 2
  store <2 x i64> %52, ptr %61, align 8, !tbaa !10
  %62 = getelementptr inbounds i64, ptr %60, i64 4
  store <2 x i64> %54, ptr %62, align 8, !tbaa !10
  %63 = getelementptr inbounds i64, ptr %60, i64 6
  store <2 x i64> %56, ptr %63, align 8, !tbaa !10
  %64 = add nuw i64 %58, 8
  %65 = icmp eq i64 %64, %47
  br i1 %65, label %66, label %57, !llvm.loop !15

66:                                               ; preds = %57
  %67 = icmp eq i64 %41, %47
  br i1 %67, label %71, label %68

68:                                               ; preds = %43, %66
  %69 = phi i64 [ %39, %43 ], [ %48, %66 ]
  br label %75

70:                                               ; preds = %71
  br i1 %11, label %87, label %80

71:                                               ; preds = %75, %66, %37
  %72 = phi i64 [ %39, %37 ], [ %44, %66 ], [ %44, %75 ]
  %73 = add nuw nsw i64 %38, 1
  %74 = icmp eq i64 %73, 1000
  br i1 %74, label %70, label %37, !llvm.loop !18

75:                                               ; preds = %68, %75
  %76 = phi i64 [ %77, %75 ], [ %69, %68 ]
  %77 = add i64 %76, 1
  %78 = getelementptr inbounds i64, ptr %10, i64 %76
  store i64 %38, ptr %78, align 8, !tbaa !10
  %79 = icmp eq i64 %77, %44
  br i1 %79, label %71, label %75, !llvm.loop !19

80:                                               ; preds = %87, %70
  %81 = phi i64 [ 0, %70 ], [ %93, %87 ]
  %82 = load i64, ptr %10, align 8, !tbaa !10
  %83 = add nsw i64 %8, -1
  %84 = getelementptr inbounds i64, ptr %10, i64 %83
  %85 = load i64, ptr %84, align 8, !tbaa !10
  %86 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %82, i64 noundef %85, i64 noundef %81)
  tail call void @free(ptr noundef %10)
  call void @llvm.lifetime.end.p0(i64 8000, ptr nonnull %3) #8
  br label %96

87:                                               ; preds = %70, %87
  %88 = phi i64 [ %94, %87 ], [ 0, %70 ]
  %89 = phi i64 [ %93, %87 ], [ 0, %70 ]
  %90 = getelementptr inbounds i64, ptr %10, i64 %88
  %91 = load i64, ptr %90, align 8, !tbaa !10
  %92 = add nsw i64 %91, %89
  %93 = srem i64 %92, 1000000007
  %94 = add nuw nsw i64 %88, 1
  %95 = icmp eq i64 %94, %8
  br i1 %95, label %80, label %87, !llvm.loop !20

96:                                               ; preds = %2, %80
  %97 = phi i32 [ 0, %80 ], [ 1, %2 ]
  ret i32 %97
}

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.start.p0(i64 immarg, ptr nocapture) #1

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #2

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #3

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.end.p0(i64 immarg, ptr nocapture) #1

; Function Attrs: mustprogress nocallback nofree nounwind willreturn memory(argmem: write)
declare void @llvm.memset.p0.i64(ptr nocapture writeonly, i8, i64, i1 immarg) #4

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #5

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #6

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite) }
attributes #2 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nocallback nofree nounwind willreturn memory(argmem: write) }
attributes #5 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #6 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #7 = { allocsize(0) }
attributes #8 = { nounwind }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 2, !"SDK Version", [2 x i32] [i32 15, i32 2]}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"uwtable", i32 1}
!4 = !{i32 7, !"frame-pointer", i32 1}
!5 = !{!"Apple clang version 16.0.0 (clang-1600.0.26.6)"}
!6 = !{!7, !7, i64 0}
!7 = !{!"any pointer", !8, i64 0}
!8 = !{!"omnipotent char", !9, i64 0}
!9 = !{!"Simple C/C++ TBAA"}
!10 = !{!11, !11, i64 0}
!11 = !{!"long", !8, i64 0}
!12 = distinct !{!12, !13}
!13 = !{!"llvm.loop.mustprogress"}
!14 = distinct !{!14, !13}
!15 = distinct !{!15, !13, !16, !17}
!16 = !{!"llvm.loop.isvectorized", i32 1}
!17 = !{!"llvm.loop.unroll.runtime.disable"}
!18 = distinct !{!18, !13}
!19 = distinct !{!19, !13, !17, !16}
!20 = distinct !{!20, !13}
