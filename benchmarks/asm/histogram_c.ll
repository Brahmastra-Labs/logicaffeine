; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/histogram/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/histogram/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [13 x i8] c"%ld %ld %ld\0A\00", align 1

; Function Attrs: nofree nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = alloca [1000 x i64], align 8
  %4 = icmp slt i32 %0, 2
  br i1 %4, label %44, label %5

5:                                                ; preds = %2
  %6 = getelementptr inbounds ptr, ptr %1, i64 1
  %7 = load ptr, ptr %6, align 8, !tbaa !6
  %8 = tail call i64 @atol(ptr nocapture noundef %7)
  call void @llvm.lifetime.start.p0(i64 8000, ptr nonnull %3) #6
  call void @llvm.memset.p0.i64(ptr noundef nonnull align 8 dereferenceable(8000) %3, i8 0, i64 8000, i1 false)
  %9 = icmp sgt i64 %8, 0
  br i1 %9, label %11, label %10

10:                                               ; preds = %11, %5
  br label %29

11:                                               ; preds = %5, %11
  %12 = phi i64 [ %25, %11 ], [ 0, %5 ]
  %13 = phi i64 [ %16, %11 ], [ 42, %5 ]
  %14 = mul nuw nsw i64 %13, 1103515245
  %15 = add nuw nsw i64 %14, 12345
  %16 = and i64 %15, 2147483647
  %17 = lshr i64 %15, 16
  %18 = trunc i64 %17 to i16
  %19 = and i16 %18, 32767
  %20 = urem i16 %19, 1000
  %21 = zext i16 %20 to i64
  %22 = getelementptr inbounds [1000 x i64], ptr %3, i64 0, i64 %21
  %23 = load i64, ptr %22, align 8, !tbaa !10
  %24 = add nsw i64 %23, 1
  store i64 %24, ptr %22, align 8, !tbaa !10
  %25 = add nuw nsw i64 %12, 1
  %26 = icmp eq i64 %25, %8
  br i1 %26, label %10, label %11, !llvm.loop !12

27:                                               ; preds = %29
  %28 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %40, i64 noundef %41, i64 noundef %38)
  call void @llvm.lifetime.end.p0(i64 8000, ptr nonnull %3) #6
  br label %44

29:                                               ; preds = %10, %29
  %30 = phi i64 [ %42, %29 ], [ 0, %10 ]
  %31 = phi i64 [ %38, %29 ], [ 0, %10 ]
  %32 = phi i64 [ %41, %29 ], [ 0, %10 ]
  %33 = phi i64 [ %40, %29 ], [ 0, %10 ]
  %34 = getelementptr inbounds [1000 x i64], ptr %3, i64 0, i64 %30
  %35 = load i64, ptr %34, align 8, !tbaa !10
  %36 = icmp sgt i64 %35, 0
  %37 = zext i1 %36 to i64
  %38 = add nuw nsw i64 %31, %37
  %39 = icmp sgt i64 %35, %33
  %40 = tail call i64 @llvm.smax.i64(i64 %35, i64 %33)
  %41 = select i1 %39, i64 %30, i64 %32
  %42 = add nuw nsw i64 %30, 1
  %43 = icmp eq i64 %42, 1000
  br i1 %43, label %27, label %29, !llvm.loop !14

44:                                               ; preds = %2, %27
  %45 = phi i32 [ 0, %27 ], [ 1, %2 ]
  ret i32 %45
}

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.start.p0(i64 immarg, ptr nocapture) #1

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #2

; Function Attrs: mustprogress nocallback nofree nounwind willreturn memory(argmem: write)
declare void @llvm.memset.p0.i64(ptr nocapture writeonly, i8, i64, i1 immarg) #3

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.end.p0(i64 immarg, ptr nocapture) #1

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #4

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i64 @llvm.smax.i64(i64, i64) #5

attributes #0 = { nofree nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite) }
attributes #2 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { mustprogress nocallback nofree nounwind willreturn memory(argmem: write) }
attributes #4 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #6 = { nounwind }

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
