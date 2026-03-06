; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/strings/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/strings/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@__stderrp = external local_unnamed_addr global ptr, align 8
@.str = private unnamed_addr constant [20 x i8] c"Usage: strings <n>\0A\00", align 1
@.str.1 = private unnamed_addr constant [4 x i8] c"%d \00", align 1
@.str.2 = private unnamed_addr constant [4 x i8] c"%d\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = alloca [32 x i8], align 1
  %4 = icmp slt i32 %0, 2
  br i1 %4, label %5, label %8

5:                                                ; preds = %2
  %6 = load ptr, ptr @__stderrp, align 8, !tbaa !6
  %7 = tail call i64 @fwrite(ptr nonnull @.str, i64 19, i64 1, ptr %6)
  br label %125

8:                                                ; preds = %2
  %9 = getelementptr inbounds ptr, ptr %1, i64 1
  %10 = load ptr, ptr %9, align 8, !tbaa !6
  %11 = tail call i32 @atoi(ptr nocapture noundef %10)
  %12 = tail call dereferenceable_or_null(16) ptr @malloc(i64 noundef 16) #10
  %13 = icmp eq ptr %12, null
  br i1 %13, label %125, label %14

14:                                               ; preds = %8
  call void @llvm.lifetime.start.p0(i64 32, ptr nonnull %3) #11
  %15 = icmp sgt i32 %11, 0
  br i1 %15, label %16, label %109

16:                                               ; preds = %14, %33
  %17 = phi i1 [ %36, %33 ], [ true, %14 ]
  %18 = phi i32 [ %35, %33 ], [ 0, %14 ]
  %19 = phi ptr [ %27, %33 ], [ %12, %14 ]
  %20 = phi i64 [ %24, %33 ], [ 0, %14 ]
  %21 = phi i64 [ %26, %33 ], [ 16, %14 ]
  %22 = call i32 (ptr, i64, ptr, ...) @snprintf(ptr nonnull dereferenceable(1) %3, i64 32, ptr nonnull @.str.1, i32 %18)
  %23 = sext i32 %22 to i64
  %24 = add i64 %20, %23
  br label %25

25:                                               ; preds = %29, %16
  %26 = phi i64 [ %21, %16 ], [ %30, %29 ]
  %27 = phi ptr [ %19, %16 ], [ %31, %29 ]
  %28 = icmp ult i64 %24, %26
  br i1 %28, label %33, label %29

29:                                               ; preds = %25
  %30 = shl i64 %26, 1
  %31 = tail call ptr @realloc(ptr noundef %27, i64 noundef %30) #12
  %32 = icmp eq ptr %31, null
  br i1 %32, label %38, label %25, !llvm.loop !10

33:                                               ; preds = %25
  %34 = getelementptr inbounds i8, ptr %27, i64 %20
  call void @llvm.memcpy.p0.p0.i64(ptr noundef align 1 %34, ptr noundef nonnull align 1 %3, i64 noundef %23, i1 noundef false) #11
  %35 = add nuw nsw i32 %18, 1
  %36 = icmp slt i32 %35, %11
  %37 = icmp eq i32 %35, %11
  br i1 %37, label %38, label %16, !llvm.loop !12

38:                                               ; preds = %33, %29
  %39 = phi i64 [ %20, %29 ], [ %24, %33 ]
  %40 = phi i1 [ %17, %29 ], [ %36, %33 ]
  %41 = phi ptr [ null, %29 ], [ %27, %33 ]
  %42 = phi i32 [ 1, %29 ], [ 0, %33 ]
  br i1 %40, label %123, label %43

43:                                               ; preds = %38
  %44 = icmp eq i64 %39, 0
  br i1 %44, label %109, label %45

45:                                               ; preds = %43
  %46 = icmp ult i64 %39, 8
  br i1 %46, label %106, label %47

47:                                               ; preds = %45
  %48 = icmp ult i64 %39, 64
  br i1 %48, label %88, label %49

49:                                               ; preds = %47
  %50 = and i64 %39, -64
  br label %51

51:                                               ; preds = %51, %49
  %52 = phi i64 [ 0, %49 ], [ %77, %51 ]
  %53 = phi <16 x i32> [ zeroinitializer, %49 ], [ %73, %51 ]
  %54 = phi <16 x i32> [ zeroinitializer, %49 ], [ %74, %51 ]
  %55 = phi <16 x i32> [ zeroinitializer, %49 ], [ %75, %51 ]
  %56 = phi <16 x i32> [ zeroinitializer, %49 ], [ %76, %51 ]
  %57 = getelementptr inbounds i8, ptr %41, i64 %52
  %58 = load <16 x i8>, ptr %57, align 1, !tbaa !13
  %59 = getelementptr inbounds i8, ptr %57, i64 16
  %60 = load <16 x i8>, ptr %59, align 1, !tbaa !13
  %61 = getelementptr inbounds i8, ptr %57, i64 32
  %62 = load <16 x i8>, ptr %61, align 1, !tbaa !13
  %63 = getelementptr inbounds i8, ptr %57, i64 48
  %64 = load <16 x i8>, ptr %63, align 1, !tbaa !13
  %65 = icmp eq <16 x i8> %58, <i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32>
  %66 = icmp eq <16 x i8> %60, <i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32>
  %67 = icmp eq <16 x i8> %62, <i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32>
  %68 = icmp eq <16 x i8> %64, <i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32>
  %69 = zext <16 x i1> %65 to <16 x i32>
  %70 = zext <16 x i1> %66 to <16 x i32>
  %71 = zext <16 x i1> %67 to <16 x i32>
  %72 = zext <16 x i1> %68 to <16 x i32>
  %73 = add <16 x i32> %53, %69
  %74 = add <16 x i32> %54, %70
  %75 = add <16 x i32> %55, %71
  %76 = add <16 x i32> %56, %72
  %77 = add nuw i64 %52, 64
  %78 = icmp eq i64 %77, %50
  br i1 %78, label %79, label %51, !llvm.loop !14

79:                                               ; preds = %51
  %80 = add <16 x i32> %74, %73
  %81 = add <16 x i32> %75, %80
  %82 = add <16 x i32> %76, %81
  %83 = tail call i32 @llvm.vector.reduce.add.v16i32(<16 x i32> %82)
  %84 = icmp eq i64 %39, %50
  br i1 %84, label %109, label %85

85:                                               ; preds = %79
  %86 = and i64 %39, 56
  %87 = icmp eq i64 %86, 0
  br i1 %87, label %106, label %88

88:                                               ; preds = %47, %85
  %89 = phi i32 [ 0, %47 ], [ %83, %85 ]
  %90 = phi i64 [ 0, %47 ], [ %50, %85 ]
  %91 = and i64 %39, -8
  %92 = insertelement <8 x i32> <i32 poison, i32 0, i32 0, i32 0, i32 0, i32 0, i32 0, i32 0>, i32 %89, i64 0
  br label %93

93:                                               ; preds = %93, %88
  %94 = phi i64 [ %90, %88 ], [ %101, %93 ]
  %95 = phi <8 x i32> [ %92, %88 ], [ %100, %93 ]
  %96 = getelementptr inbounds i8, ptr %41, i64 %94
  %97 = load <8 x i8>, ptr %96, align 1, !tbaa !13
  %98 = icmp eq <8 x i8> %97, <i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32, i8 32>
  %99 = zext <8 x i1> %98 to <8 x i32>
  %100 = add <8 x i32> %95, %99
  %101 = add nuw i64 %94, 8
  %102 = icmp eq i64 %101, %91
  br i1 %102, label %103, label %93, !llvm.loop !17

103:                                              ; preds = %93
  %104 = tail call i32 @llvm.vector.reduce.add.v8i32(<8 x i32> %100)
  %105 = icmp eq i64 %39, %91
  br i1 %105, label %109, label %106

106:                                              ; preds = %45, %85, %103
  %107 = phi i64 [ 0, %45 ], [ %50, %85 ], [ %91, %103 ]
  %108 = phi i32 [ 0, %45 ], [ %83, %85 ], [ %104, %103 ]
  br label %113

109:                                              ; preds = %113, %79, %103, %14, %43
  %110 = phi ptr [ %41, %43 ], [ %12, %14 ], [ %41, %103 ], [ %41, %79 ], [ %41, %113 ]
  %111 = phi i32 [ 0, %43 ], [ 0, %14 ], [ %104, %103 ], [ %83, %79 ], [ %120, %113 ]
  %112 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str.2, i32 noundef %111)
  tail call void @free(ptr noundef %110)
  br label %123

113:                                              ; preds = %106, %113
  %114 = phi i64 [ %121, %113 ], [ %107, %106 ]
  %115 = phi i32 [ %120, %113 ], [ %108, %106 ]
  %116 = getelementptr inbounds i8, ptr %41, i64 %114
  %117 = load i8, ptr %116, align 1, !tbaa !13
  %118 = icmp eq i8 %117, 32
  %119 = zext i1 %118 to i32
  %120 = add nuw nsw i32 %115, %119
  %121 = add nuw i64 %114, 1
  %122 = icmp eq i64 %121, %39
  br i1 %122, label %109, label %113, !llvm.loop !18

123:                                              ; preds = %38, %109
  %124 = phi i32 [ 0, %109 ], [ %42, %38 ]
  call void @llvm.lifetime.end.p0(i64 32, ptr nonnull %3) #11
  br label %125

125:                                              ; preds = %123, %8, %5
  %126 = phi i32 [ 1, %5 ], [ %124, %123 ], [ 1, %8 ]
  ret i32 %126
}

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.start.p0(i64 immarg, ptr nocapture) #1

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i32 @atoi(ptr nocapture noundef) local_unnamed_addr #2

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #3

; Function Attrs: mustprogress nounwind willreturn allockind("realloc") allocsize(1) memory(argmem: readwrite, inaccessiblemem: readwrite)
declare noalias noundef ptr @realloc(ptr allocptr nocapture noundef, i64 noundef) local_unnamed_addr #4

; Function Attrs: mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite)
declare void @llvm.lifetime.end.p0(i64 immarg, ptr nocapture) #1

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #5

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #6

; Function Attrs: nofree nounwind
declare noundef i32 @snprintf(ptr noalias nocapture noundef writeonly, i64 noundef, ptr nocapture noundef readonly, ...) local_unnamed_addr #7

; Function Attrs: nofree nounwind
declare noundef i64 @fwrite(ptr nocapture noundef, i64 noundef, i64 noundef, ptr nocapture noundef) local_unnamed_addr #7

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i32 @llvm.vector.reduce.add.v16i32(<16 x i32>) #8

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i32 @llvm.vector.reduce.add.v8i32(<8 x i32>) #8

; Function Attrs: nocallback nofree nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias nocapture writeonly, ptr noalias nocapture readonly, i64, i1 immarg) #9

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nocallback nofree nosync nounwind willreturn memory(argmem: readwrite) }
attributes #2 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nounwind willreturn allockind("realloc") allocsize(1) memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #6 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #7 = { nofree nounwind }
attributes #8 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #9 = { nocallback nofree nounwind willreturn memory(argmem: readwrite) }
attributes #10 = { allocsize(0) }
attributes #11 = { nounwind }
attributes #12 = { allocsize(1) }

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
!12 = distinct !{!12, !11}
!13 = !{!8, !8, i64 0}
!14 = distinct !{!14, !11, !15, !16}
!15 = !{!"llvm.loop.isvectorized", i32 1}
!16 = !{!"llvm.loop.unroll.runtime.disable"}
!17 = distinct !{!17, !11, !15, !16}
!18 = distinct !{!18, !11, !16, !15}
