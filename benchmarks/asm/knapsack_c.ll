; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/knapsack/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/knapsack/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %66, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = mul i64 %7, 5
  %9 = shl i64 %7, 3
  %10 = tail call ptr @malloc(i64 noundef %9) #7
  %11 = tail call ptr @malloc(i64 noundef %9) #7
  %12 = icmp sgt i64 %7, 0
  br i1 %12, label %19, label %13

13:                                               ; preds = %19, %4
  %14 = add nsw i64 %8, 1
  %15 = tail call ptr @calloc(i64 noundef %14, i64 noundef 8) #8
  %16 = tail call ptr @calloc(i64 noundef %14, i64 noundef 8) #8
  br i1 %12, label %17, label %39

17:                                               ; preds = %13
  %18 = tail call i64 @llvm.smax.i64(i64 %8, i64 0)
  br label %33

19:                                               ; preds = %4, %19
  %20 = phi i64 [ %31, %19 ], [ 0, %4 ]
  %21 = mul nsw i64 %20, 17
  %22 = add nuw nsw i64 %21, 3
  %23 = urem i64 %22, 50
  %24 = add nuw nsw i64 %23, 1
  %25 = getelementptr inbounds i64, ptr %10, i64 %20
  store i64 %24, ptr %25, align 8, !tbaa !10
  %26 = mul nsw i64 %20, 31
  %27 = add nuw nsw i64 %26, 7
  %28 = urem i64 %27, 100
  %29 = add nuw nsw i64 %28, 1
  %30 = getelementptr inbounds i64, ptr %11, i64 %20
  store i64 %29, ptr %30, align 8, !tbaa !10
  %31 = add nuw nsw i64 %20, 1
  %32 = icmp eq i64 %31, %7
  br i1 %32, label %13, label %19, !llvm.loop !12

33:                                               ; preds = %45, %17
  %34 = phi i64 [ 0, %17 ], [ %46, %45 ]
  %35 = phi ptr [ %16, %17 ], [ %36, %45 ]
  %36 = phi ptr [ %15, %17 ], [ %35, %45 ]
  %37 = getelementptr inbounds i64, ptr %10, i64 %34
  %38 = getelementptr inbounds i64, ptr %11, i64 %34
  br label %48

39:                                               ; preds = %45, %13
  %40 = phi ptr [ %15, %13 ], [ %35, %45 ]
  %41 = phi ptr [ %16, %13 ], [ %36, %45 ]
  %42 = getelementptr inbounds i64, ptr %40, i64 %8
  %43 = load i64, ptr %42, align 8, !tbaa !10
  %44 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %43)
  tail call void @free(ptr noundef %10)
  tail call void @free(ptr noundef %11)
  tail call void @free(ptr noundef %40)
  tail call void @free(ptr noundef %41)
  br label %66

45:                                               ; preds = %63
  %46 = add nuw nsw i64 %34, 1
  %47 = icmp eq i64 %46, %7
  br i1 %47, label %39, label %33, !llvm.loop !14

48:                                               ; preds = %33, %63
  %49 = phi i64 [ 0, %33 ], [ %64, %63 ]
  %50 = getelementptr inbounds i64, ptr %36, i64 %49
  %51 = load i64, ptr %50, align 8, !tbaa !10
  %52 = getelementptr inbounds i64, ptr %35, i64 %49
  store i64 %51, ptr %52, align 8, !tbaa !10
  %53 = load i64, ptr %37, align 8, !tbaa !10
  %54 = icmp slt i64 %49, %53
  br i1 %54, label %63, label %55

55:                                               ; preds = %48
  %56 = sub nsw i64 %49, %53
  %57 = getelementptr inbounds i64, ptr %36, i64 %56
  %58 = load i64, ptr %57, align 8, !tbaa !10
  %59 = load i64, ptr %38, align 8, !tbaa !10
  %60 = add nsw i64 %59, %58
  %61 = icmp sgt i64 %60, %51
  br i1 %61, label %62, label %63

62:                                               ; preds = %55
  store i64 %60, ptr %52, align 8, !tbaa !10
  br label %63

63:                                               ; preds = %48, %55, %62
  %64 = add nuw i64 %49, 1
  %65 = icmp eq i64 %49, %18
  br i1 %65, label %45, label %48, !llvm.loop !15

66:                                               ; preds = %2, %39
  %67 = phi i32 [ 0, %39 ], [ 1, %2 ]
  ret i32 %67
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
declare i64 @llvm.smax.i64(i64, i64) #6

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
!10 = !{!11, !11, i64 0}
!11 = !{!"long", !8, i64 0}
!12 = distinct !{!12, !13}
!13 = !{!"llvm.loop.mustprogress"}
!14 = distinct !{!14, !13}
!15 = distinct !{!15, !13}
