; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/array_fill/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/array_fill/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %63, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = shl i64 %7, 3
  %9 = tail call ptr @malloc(i64 noundef %8) #5
  %10 = icmp sgt i64 %7, 0
  br i1 %10, label %11, label %51

11:                                               ; preds = %4
  %12 = icmp ult i64 %7, 4
  br i1 %12, label %40, label %13

13:                                               ; preds = %11
  %14 = and i64 %7, -4
  br label %15

15:                                               ; preds = %15, %13
  %16 = phi i64 [ 0, %13 ], [ %36, %15 ]
  %17 = or i64 %16, 1
  %18 = or i64 %16, 2
  %19 = or i64 %16, 3
  %20 = mul nsw i64 %16, 7
  %21 = mul nsw i64 %17, 7
  %22 = mul nsw i64 %18, 7
  %23 = mul nsw i64 %19, 7
  %24 = or i64 %20, 3
  %25 = add nuw nsw i64 %21, 3
  %26 = add nuw nsw i64 %22, 3
  %27 = add nuw nsw i64 %23, 3
  %28 = urem i64 %24, 1000000
  %29 = urem i64 %25, 1000000
  %30 = urem i64 %26, 1000000
  %31 = urem i64 %27, 1000000
  %32 = getelementptr inbounds i64, ptr %9, i64 %16
  %33 = getelementptr inbounds i64, ptr %9, i64 %17
  %34 = getelementptr inbounds i64, ptr %9, i64 %18
  %35 = getelementptr inbounds i64, ptr %9, i64 %19
  store i64 %28, ptr %32, align 8, !tbaa !10
  store i64 %29, ptr %33, align 8, !tbaa !10
  store i64 %30, ptr %34, align 8, !tbaa !10
  store i64 %31, ptr %35, align 8, !tbaa !10
  %36 = add nuw i64 %16, 4
  %37 = icmp eq i64 %36, %14
  br i1 %37, label %38, label %15, !llvm.loop !12

38:                                               ; preds = %15
  %39 = icmp eq i64 %7, %14
  br i1 %39, label %42, label %40

40:                                               ; preds = %11, %38
  %41 = phi i64 [ 0, %11 ], [ %14, %38 ]
  br label %43

42:                                               ; preds = %43, %38
  br i1 %10, label %54, label %51

43:                                               ; preds = %40, %43
  %44 = phi i64 [ %49, %43 ], [ %41, %40 ]
  %45 = mul nsw i64 %44, 7
  %46 = add nuw nsw i64 %45, 3
  %47 = urem i64 %46, 1000000
  %48 = getelementptr inbounds i64, ptr %9, i64 %44
  store i64 %47, ptr %48, align 8, !tbaa !10
  %49 = add nuw nsw i64 %44, 1
  %50 = icmp eq i64 %49, %7
  br i1 %50, label %42, label %43, !llvm.loop !16

51:                                               ; preds = %54, %4, %42
  %52 = phi i64 [ 0, %42 ], [ 0, %4 ], [ %60, %54 ]
  %53 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %52)
  tail call void @free(ptr noundef %9)
  br label %63

54:                                               ; preds = %42, %54
  %55 = phi i64 [ %61, %54 ], [ 0, %42 ]
  %56 = phi i64 [ %60, %54 ], [ 0, %42 ]
  %57 = getelementptr inbounds i64, ptr %9, i64 %55
  %58 = load i64, ptr %57, align 8, !tbaa !10
  %59 = add nsw i64 %58, %56
  %60 = srem i64 %59, 1000000007
  %61 = add nuw nsw i64 %55, 1
  %62 = icmp eq i64 %61, %7
  br i1 %62, label %51, label %54, !llvm.loop !17

63:                                               ; preds = %2, %51
  %64 = phi i32 [ 0, %51 ], [ 1, %2 ]
  ret i32 %64
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #4

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { allocsize(0) }

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
!12 = distinct !{!12, !13, !14, !15}
!13 = !{!"llvm.loop.mustprogress"}
!14 = !{!"llvm.loop.isvectorized", i32 1}
!15 = !{!"llvm.loop.unroll.runtime.disable"}
!16 = distinct !{!16, !13, !14}
!17 = distinct !{!17, !13}
