; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/two_sum/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/two_sum/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

%struct.Entry = type { i64, i32 }

@.str = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nofree norecurse nosync nounwind ssp memory(argmem: read) uwtable(sync)
define i32 @ht_contains(ptr nocapture noundef readonly %0, i64 noundef %1, i64 noundef %2) local_unnamed_addr #0 {
  %4 = mul i64 %2, 2654435761
  %5 = and i64 %4, %1
  %6 = getelementptr inbounds %struct.Entry, ptr %0, i64 %5, i32 1
  %7 = load i32, ptr %6, align 8, !tbaa !6
  %8 = icmp eq i32 %7, 0
  br i1 %8, label %20, label %15

9:                                                ; preds = %15
  %10 = add i64 %16, 1
  %11 = and i64 %10, %1
  %12 = getelementptr inbounds %struct.Entry, ptr %0, i64 %11, i32 1
  %13 = load i32, ptr %12, align 8, !tbaa !6
  %14 = icmp eq i32 %13, 0
  br i1 %14, label %20, label %15, !llvm.loop !12

15:                                               ; preds = %3, %9
  %16 = phi i64 [ %11, %9 ], [ %5, %3 ]
  %17 = getelementptr inbounds %struct.Entry, ptr %0, i64 %16
  %18 = load i64, ptr %17, align 8, !tbaa !14
  %19 = icmp eq i64 %18, %2
  br i1 %19, label %20, label %9

20:                                               ; preds = %15, %9, %3
  %21 = phi i32 [ 0, %3 ], [ 0, %9 ], [ 1, %15 ]
  ret i32 %21
}

; Function Attrs: nofree norecurse nosync nounwind ssp memory(readwrite, inaccessiblemem: none) uwtable(sync)
define void @ht_insert(ptr nocapture noundef %0, i64 noundef %1, i64 noundef %2) local_unnamed_addr #1 {
  %4 = mul i64 %2, 2654435761
  %5 = and i64 %4, %1
  %6 = getelementptr inbounds %struct.Entry, ptr %0, i64 %5
  %7 = getelementptr inbounds %struct.Entry, ptr %0, i64 %5, i32 1
  %8 = load i32, ptr %7, align 8, !tbaa !6
  %9 = icmp eq i32 %8, 0
  br i1 %9, label %22, label %17

10:                                               ; preds = %17
  %11 = add i64 %19, 1
  %12 = and i64 %11, %1
  %13 = getelementptr inbounds %struct.Entry, ptr %0, i64 %12
  %14 = getelementptr inbounds %struct.Entry, ptr %0, i64 %12, i32 1
  %15 = load i32, ptr %14, align 8, !tbaa !6
  %16 = icmp eq i32 %15, 0
  br i1 %16, label %22, label %17, !llvm.loop !15

17:                                               ; preds = %3, %10
  %18 = phi ptr [ %13, %10 ], [ %6, %3 ]
  %19 = phi i64 [ %12, %10 ], [ %5, %3 ]
  %20 = load i64, ptr %18, align 8, !tbaa !14
  %21 = icmp eq i64 %20, %2
  br i1 %21, label %25, label %10

22:                                               ; preds = %10, %3
  %23 = phi ptr [ %6, %3 ], [ %13, %10 ]
  %24 = phi ptr [ %7, %3 ], [ %14, %10 ]
  store i64 %2, ptr %23, align 8, !tbaa !14
  store i32 1, ptr %24, align 8, !tbaa !6
  br label %25

25:                                               ; preds = %17, %22
  ret void
}

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #2 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %97, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !16
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = shl i64 %7, 3
  %9 = tail call ptr @malloc(i64 noundef %8) #9
  %10 = icmp sgt i64 %7, 0
  br i1 %10, label %30, label %11

11:                                               ; preds = %30, %4
  %12 = shl nsw i64 %7, 1
  %13 = add i64 %12, -1
  %14 = lshr i64 %13, 1
  %15 = or i64 %14, %13
  %16 = lshr i64 %15, 2
  %17 = or i64 %16, %15
  %18 = lshr i64 %17, 4
  %19 = or i64 %18, %17
  %20 = lshr i64 %19, 8
  %21 = or i64 %20, %19
  %22 = lshr i64 %21, 16
  %23 = or i64 %22, %21
  %24 = lshr i64 %23, 32
  %25 = or i64 %24, %23
  %26 = add i64 %25, 1
  %27 = tail call i64 @llvm.umax.i64(i64 %26, i64 16)
  %28 = add i64 %27, -1
  %29 = tail call ptr @calloc(i64 noundef %27, i64 noundef 16) #10
  br i1 %10, label %45, label %42

30:                                               ; preds = %4, %30
  %31 = phi i64 [ %35, %30 ], [ 42, %4 ]
  %32 = phi i64 [ %40, %30 ], [ 0, %4 ]
  %33 = mul nuw nsw i64 %31, 1103515245
  %34 = add nuw nsw i64 %33, 12345
  %35 = and i64 %34, 2147483647
  %36 = lshr i64 %34, 16
  %37 = and i64 %36, 32767
  %38 = srem i64 %37, %7
  %39 = getelementptr inbounds i64, ptr %9, i64 %32
  store i64 %38, ptr %39, align 8, !tbaa !18
  %40 = add nuw nsw i64 %32, 1
  %41 = icmp eq i64 %40, %7
  br i1 %41, label %11, label %30, !llvm.loop !19

42:                                               ; preds = %94, %11
  %43 = phi i64 [ 0, %11 ], [ %72, %94 ]
  %44 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %43)
  tail call void @free(ptr noundef %9)
  tail call void @free(ptr noundef %29)
  br label %97

45:                                               ; preds = %11, %94
  %46 = phi i64 [ %95, %94 ], [ 0, %11 ]
  %47 = phi i64 [ %72, %94 ], [ 0, %11 ]
  %48 = getelementptr inbounds i64, ptr %9, i64 %46
  %49 = load i64, ptr %48, align 8, !tbaa !18
  %50 = sub nsw i64 %7, %49
  %51 = icmp sgt i64 %50, -1
  br i1 %51, label %52, label %71

52:                                               ; preds = %45
  %53 = mul i64 %50, 2654435761
  %54 = and i64 %53, %28
  %55 = getelementptr inbounds %struct.Entry, ptr %29, i64 %54, i32 1
  %56 = load i32, ptr %55, align 8, !tbaa !6
  %57 = icmp eq i32 %56, 0
  br i1 %57, label %71, label %64

58:                                               ; preds = %64
  %59 = add nuw i64 %65, 1
  %60 = and i64 %59, %28
  %61 = getelementptr inbounds %struct.Entry, ptr %29, i64 %60, i32 1
  %62 = load i32, ptr %61, align 8, !tbaa !6
  %63 = icmp eq i32 %62, 0
  br i1 %63, label %71, label %64, !llvm.loop !12

64:                                               ; preds = %52, %58
  %65 = phi i64 [ %60, %58 ], [ %54, %52 ]
  %66 = getelementptr inbounds %struct.Entry, ptr %29, i64 %65
  %67 = load i64, ptr %66, align 8, !tbaa !14
  %68 = icmp eq i64 %67, %50
  br i1 %68, label %69, label %58

69:                                               ; preds = %64
  %70 = add nsw i64 %47, 1
  br label %71

71:                                               ; preds = %58, %69, %52, %45
  %72 = phi i64 [ %47, %45 ], [ %70, %69 ], [ %47, %52 ], [ %47, %58 ]
  %73 = mul i64 %49, 2654435761
  %74 = and i64 %73, %28
  %75 = getelementptr inbounds %struct.Entry, ptr %29, i64 %74
  %76 = getelementptr inbounds %struct.Entry, ptr %29, i64 %74, i32 1
  %77 = load i32, ptr %76, align 8, !tbaa !6
  %78 = icmp eq i32 %77, 0
  br i1 %78, label %91, label %86

79:                                               ; preds = %86
  %80 = add nuw i64 %88, 1
  %81 = and i64 %80, %28
  %82 = getelementptr inbounds %struct.Entry, ptr %29, i64 %81
  %83 = getelementptr inbounds %struct.Entry, ptr %29, i64 %81, i32 1
  %84 = load i32, ptr %83, align 8, !tbaa !6
  %85 = icmp eq i32 %84, 0
  br i1 %85, label %91, label %86, !llvm.loop !15

86:                                               ; preds = %71, %79
  %87 = phi ptr [ %82, %79 ], [ %75, %71 ]
  %88 = phi i64 [ %81, %79 ], [ %74, %71 ]
  %89 = load i64, ptr %87, align 8, !tbaa !14
  %90 = icmp eq i64 %89, %49
  br i1 %90, label %94, label %79

91:                                               ; preds = %79, %71
  %92 = phi ptr [ %75, %71 ], [ %82, %79 ]
  %93 = phi ptr [ %76, %71 ], [ %83, %79 ]
  store i64 %49, ptr %92, align 8, !tbaa !14
  store i32 1, ptr %93, align 8, !tbaa !6
  br label %94

94:                                               ; preds = %86, %91
  %95 = add nuw nsw i64 %46, 1
  %96 = icmp eq i64 %95, %7
  br i1 %96, label %42, label %45, !llvm.loop !20

97:                                               ; preds = %2, %42
  %98 = phi i32 [ 0, %42 ], [ 1, %2 ]
  ret i32 %98
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #3

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #4

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @calloc(i64 noundef, i64 noundef) local_unnamed_addr #5

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #6

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #7

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i64 @llvm.umax.i64(i64, i64) #8

attributes #0 = { nofree norecurse nosync nounwind ssp memory(argmem: read) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { nofree norecurse nosync nounwind ssp memory(readwrite, inaccessiblemem: none) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #6 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #7 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #8 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #9 = { allocsize(0) }
attributes #10 = { allocsize(0,1) }

!llvm.module.flags = !{!0, !1, !2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i32 2, !"SDK Version", [2 x i32] [i32 15, i32 2]}
!1 = !{i32 1, !"wchar_size", i32 4}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 7, !"uwtable", i32 1}
!4 = !{i32 7, !"frame-pointer", i32 1}
!5 = !{!"Apple clang version 16.0.0 (clang-1600.0.26.6)"}
!6 = !{!7, !11, i64 8}
!7 = !{!"Entry", !8, i64 0, !11, i64 8}
!8 = !{!"long", !9, i64 0}
!9 = !{!"omnipotent char", !10, i64 0}
!10 = !{!"Simple C/C++ TBAA"}
!11 = !{!"int", !9, i64 0}
!12 = distinct !{!12, !13}
!13 = !{!"llvm.loop.mustprogress"}
!14 = !{!7, !8, i64 0}
!15 = distinct !{!15, !13}
!16 = !{!17, !17, i64 0}
!17 = !{!"any pointer", !9, i64 0}
!18 = !{!8, !8, i64 0}
!19 = distinct !{!19, !13}
!20 = distinct !{!20, !13}
