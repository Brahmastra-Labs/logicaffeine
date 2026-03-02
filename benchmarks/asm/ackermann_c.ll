; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/ackermann/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/ackermann/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@__stderrp = external local_unnamed_addr global ptr, align 8
@.str = private unnamed_addr constant [22 x i8] c"Usage: ackermann <m>\0A\00", align 1
@.str.1 = private unnamed_addr constant [5 x i8] c"%ld\0A\00", align 1

; Function Attrs: nofree nosync nounwind ssp memory(none) uwtable(sync)
define i64 @ackermann(i64 noundef %0, i64 noundef %1) local_unnamed_addr #0 {
  %3 = icmp eq i64 %0, 0
  br i1 %3, label %4, label %7

4:                                                ; preds = %11, %2
  %5 = phi i64 [ %1, %2 ], [ %12, %11 ]
  %6 = add nsw i64 %5, 1
  ret i64 %6

7:                                                ; preds = %2, %11
  %8 = phi i64 [ %12, %11 ], [ %1, %2 ]
  %9 = phi i64 [ %13, %11 ], [ %0, %2 ]
  %10 = icmp eq i64 %8, 0
  br i1 %10, label %11, label %15

11:                                               ; preds = %7, %15
  %12 = phi i64 [ %17, %15 ], [ 1, %7 ]
  %13 = add nsw i64 %9, -1
  %14 = icmp eq i64 %13, 0
  br i1 %14, label %4, label %7

15:                                               ; preds = %7
  %16 = add nsw i64 %8, -1
  %17 = tail call i64 @ackermann(i64 noundef %9, i64 noundef %16)
  br label %11
}

; Function Attrs: nofree nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #1 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %4, label %7

4:                                                ; preds = %2
  %5 = load ptr, ptr @__stderrp, align 8, !tbaa !6
  %6 = tail call i64 @fwrite(ptr nonnull @.str, i64 21, i64 1, ptr %5)
  br label %13

7:                                                ; preds = %2
  %8 = getelementptr inbounds ptr, ptr %1, i64 1
  %9 = load ptr, ptr %8, align 8, !tbaa !6
  %10 = tail call i64 @atol(ptr nocapture noundef %9)
  %11 = tail call i64 @ackermann(i64 noundef 3, i64 noundef %10)
  %12 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.1, i64 noundef %11)
  br label %13

13:                                               ; preds = %7, %4
  %14 = phi i32 [ 1, %4 ], [ 0, %7 ]
  ret i32 %14
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i64 @atol(ptr nocapture noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: nofree nounwind
declare noundef i64 @fwrite(ptr nocapture noundef, i64 noundef, i64 noundef, ptr nocapture noundef) local_unnamed_addr #4

attributes #0 = { nofree nosync nounwind ssp memory(none) uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { nofree nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { nofree nounwind }

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
