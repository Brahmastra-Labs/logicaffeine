; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/matrix_mult/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/matrix_mult/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %76, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = mul nsw i64 %7, %7
  %9 = shl i64 %8, 3
  %10 = tail call ptr @malloc(i64 noundef %9) #7
  %11 = tail call ptr @malloc(i64 noundef %9) #7
  %12 = tail call ptr @calloc(i64 noundef %8, i64 noundef 8) #8
  %13 = icmp sgt i64 %7, 0
  br i1 %13, label %14, label %35

14:                                               ; preds = %4, %18
  %15 = phi i64 [ %19, %18 ], [ 0, %4 ]
  %16 = mul nsw i64 %15, %7
  br label %21

17:                                               ; preds = %18
  br i1 %13, label %32, label %35

18:                                               ; preds = %21
  %19 = add nuw nsw i64 %15, 1
  %20 = icmp eq i64 %19, %7
  br i1 %20, label %17, label %14, !llvm.loop !10

21:                                               ; preds = %14, %21
  %22 = phi i64 [ 0, %14 ], [ %30, %21 ]
  %23 = add nsw i64 %22, %16
  %24 = srem i64 %23, 100
  %25 = getelementptr inbounds i64, ptr %10, i64 %23
  store i64 %24, ptr %25, align 8, !tbaa !12
  %26 = mul nsw i64 %22, %7
  %27 = add nsw i64 %26, %15
  %28 = srem i64 %27, 100
  %29 = getelementptr inbounds i64, ptr %11, i64 %23
  store i64 %28, ptr %29, align 8, !tbaa !12
  %30 = add nuw nsw i64 %22, 1
  %31 = icmp eq i64 %30, %7
  br i1 %31, label %18, label %21, !llvm.loop !14

32:                                               ; preds = %17, %45
  %33 = phi i64 [ %46, %45 ], [ 0, %17 ]
  %34 = mul nsw i64 %33, %7
  br label %39

35:                                               ; preds = %45, %4, %17
  %36 = icmp eq i64 %7, 0
  br i1 %36, label %64, label %37

37:                                               ; preds = %35
  %38 = tail call i64 @llvm.umax.i64(i64 %8, i64 1)
  br label %67

39:                                               ; preds = %32, %48
  %40 = phi i64 [ 0, %32 ], [ %49, %48 ]
  %41 = add nsw i64 %40, %34
  %42 = getelementptr inbounds i64, ptr %10, i64 %41
  %43 = load i64, ptr %42, align 8, !tbaa !12
  %44 = mul nsw i64 %40, %7
  br label %51

45:                                               ; preds = %48
  %46 = add nuw nsw i64 %33, 1
  %47 = icmp eq i64 %46, %7
  br i1 %47, label %35, label %32, !llvm.loop !15

48:                                               ; preds = %51
  %49 = add nuw nsw i64 %40, 1
  %50 = icmp eq i64 %49, %7
  br i1 %50, label %45, label %39, !llvm.loop !16

51:                                               ; preds = %39, %51
  %52 = phi i64 [ 0, %39 ], [ %62, %51 ]
  %53 = add nsw i64 %52, %34
  %54 = getelementptr inbounds i64, ptr %12, i64 %53
  %55 = load i64, ptr %54, align 8, !tbaa !12
  %56 = add nsw i64 %52, %44
  %57 = getelementptr inbounds i64, ptr %11, i64 %56
  %58 = load i64, ptr %57, align 8, !tbaa !12
  %59 = mul nsw i64 %58, %43
  %60 = add nsw i64 %59, %55
  %61 = srem i64 %60, 1000000007
  store i64 %61, ptr %54, align 8, !tbaa !12
  %62 = add nuw nsw i64 %52, 1
  %63 = icmp eq i64 %62, %7
  br i1 %63, label %48, label %51, !llvm.loop !17

64:                                               ; preds = %67, %35
  %65 = phi i64 [ 0, %35 ], [ %73, %67 ]
  %66 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %65)
  tail call void @free(ptr noundef %10)
  tail call void @free(ptr noundef %11)
  tail call void @free(ptr noundef %12)
  br label %76

67:                                               ; preds = %37, %67
  %68 = phi i64 [ %74, %67 ], [ 0, %37 ]
  %69 = phi i64 [ %73, %67 ], [ 0, %37 ]
  %70 = getelementptr inbounds i64, ptr %12, i64 %68
  %71 = load i64, ptr %70, align 8, !tbaa !12
  %72 = add nsw i64 %71, %69
  %73 = srem i64 %72, 1000000007
  %74 = add nuw nsw i64 %68, 1
  %75 = icmp eq i64 %74, %38
  br i1 %75, label %64, label %67, !llvm.loop !18

76:                                               ; preds = %2, %64
  %77 = phi i32 [ 0, %64 ], [ 1, %2 ]
  ret i32 %77
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #2

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @calloc(i64 noundef, i64 noundef) local_unnamed_addr #3

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #4

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #5

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i64 @llvm.umax.i64(i64, i64) #6

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #6 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #7 = { allocsize(0) }
attributes #8 = { allocsize(0,1) }

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
!10 = distinct !{!10, !11}
!11 = !{!"llvm.loop.mustprogress"}
!12 = !{!13, !13, i64 0}
!13 = !{!"long", !8, i64 0}
!14 = distinct !{!14, !11}
!15 = distinct !{!15, !11}
!16 = distinct !{!16, !11}
!17 = distinct !{!17, !11}
!18 = distinct !{!18, !11}
