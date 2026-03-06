; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/fannkuch/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/fannkuch/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [7 x i8] c"%d\0A%d\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %156, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i32 @atoi(ptr nocapture noundef %6)
  %8 = sext i32 %7 to i64
  %9 = shl nsw i64 %8, 2
  %10 = tail call ptr @malloc(i64 noundef %9) #7
  %11 = tail call ptr @malloc(i64 noundef %9) #7
  %12 = tail call ptr @malloc(i64 noundef %9) #7
  %13 = icmp sgt i32 %7, 0
  br i1 %13, label %14, label %36

14:                                               ; preds = %4
  %15 = zext i32 %7 to i64
  %16 = icmp ult i32 %7, 16
  br i1 %16, label %34, label %17

17:                                               ; preds = %14
  %18 = and i64 %15, 4294967280
  br label %19

19:                                               ; preds = %19, %17
  %20 = phi i64 [ 0, %17 ], [ %29, %19 ]
  %21 = phi <4 x i32> [ <i32 0, i32 1, i32 2, i32 3>, %17 ], [ %30, %19 ]
  %22 = add <4 x i32> %21, <i32 4, i32 4, i32 4, i32 4>
  %23 = add <4 x i32> %21, <i32 8, i32 8, i32 8, i32 8>
  %24 = add <4 x i32> %21, <i32 12, i32 12, i32 12, i32 12>
  %25 = getelementptr inbounds i32, ptr %11, i64 %20
  store <4 x i32> %21, ptr %25, align 4, !tbaa !10
  %26 = getelementptr inbounds i32, ptr %25, i64 4
  store <4 x i32> %22, ptr %26, align 4, !tbaa !10
  %27 = getelementptr inbounds i32, ptr %25, i64 8
  store <4 x i32> %23, ptr %27, align 4, !tbaa !10
  %28 = getelementptr inbounds i32, ptr %25, i64 12
  store <4 x i32> %24, ptr %28, align 4, !tbaa !10
  %29 = add nuw i64 %20, 16
  %30 = add <4 x i32> %21, <i32 16, i32 16, i32 16, i32 16>
  %31 = icmp eq i64 %29, %18
  br i1 %31, label %32, label %19, !llvm.loop !12

32:                                               ; preds = %19
  %33 = icmp eq i64 %18, %15
  br i1 %33, label %36, label %34

34:                                               ; preds = %14, %32
  %35 = phi i64 [ 0, %14 ], [ %18, %32 ]
  br label %38

36:                                               ; preds = %38, %32, %4
  %37 = getelementptr i8, ptr %11, i64 4
  br label %44

38:                                               ; preds = %34, %38
  %39 = phi i64 [ %42, %38 ], [ %35, %34 ]
  %40 = getelementptr inbounds i32, ptr %11, i64 %39
  %41 = trunc i64 %39 to i32
  store i32 %41, ptr %40, align 4, !tbaa !10
  %42 = add nuw nsw i64 %39, 1
  %43 = icmp eq i64 %42, %15
  br i1 %43, label %36, label %38, !llvm.loop !16

44:                                               ; preds = %147, %36
  %45 = phi i32 [ 0, %36 ], [ %133, %147 ]
  %46 = phi i32 [ 0, %36 ], [ %134, %147 ]
  %47 = phi i32 [ %7, %36 ], [ %139, %147 ]
  %48 = phi i32 [ 0, %36 ], [ %128, %147 ]
  %49 = icmp sgt i32 %47, 1
  br i1 %49, label %50, label %99

50:                                               ; preds = %44
  %51 = zext i32 %47 to i64
  %52 = add nsw i64 %51, -1
  %53 = icmp ult i32 %47, 25
  br i1 %53, label %90, label %54

54:                                               ; preds = %50
  %55 = add nsw i64 %51, -2
  %56 = add i32 %47, -1
  %57 = trunc i64 %55 to i32
  %58 = icmp ult i32 %56, %57
  %59 = icmp ugt i64 %55, 4294967295
  %60 = or i1 %58, %59
  br i1 %60, label %90, label %61

61:                                               ; preds = %54
  %62 = and i64 %52, -16
  %63 = sub nsw i64 %51, %62
  %64 = insertelement <4 x i32> poison, i32 %47, i64 0
  %65 = shufflevector <4 x i32> %64, <4 x i32> poison, <4 x i32> zeroinitializer
  %66 = add <4 x i32> %65, <i32 0, i32 -1, i32 -2, i32 -3>
  br label %67

67:                                               ; preds = %67, %61
  %68 = phi i64 [ 0, %61 ], [ %85, %67 ]
  %69 = phi <4 x i32> [ %66, %61 ], [ %86, %67 ]
  %70 = add <4 x i32> %69, <i32 -4, i32 -4, i32 -4, i32 -4>
  %71 = add <4 x i32> %69, <i32 -8, i32 -8, i32 -8, i32 -8>
  %72 = add <4 x i32> %69, <i32 -12, i32 -12, i32 -12, i32 -12>
  %73 = xor i64 %68, -1
  %74 = add i64 %73, %51
  %75 = and i64 %74, 4294967295
  %76 = getelementptr inbounds i32, ptr %12, i64 %75
  %77 = shufflevector <4 x i32> %69, <4 x i32> poison, <4 x i32> <i32 3, i32 2, i32 1, i32 0>
  %78 = getelementptr inbounds i32, ptr %76, i64 -3
  store <4 x i32> %77, ptr %78, align 4, !tbaa !10
  %79 = shufflevector <4 x i32> %70, <4 x i32> poison, <4 x i32> <i32 3, i32 2, i32 1, i32 0>
  %80 = getelementptr inbounds i32, ptr %76, i64 -7
  store <4 x i32> %79, ptr %80, align 4, !tbaa !10
  %81 = shufflevector <4 x i32> %71, <4 x i32> poison, <4 x i32> <i32 3, i32 2, i32 1, i32 0>
  %82 = getelementptr inbounds i32, ptr %76, i64 -11
  store <4 x i32> %81, ptr %82, align 4, !tbaa !10
  %83 = shufflevector <4 x i32> %72, <4 x i32> poison, <4 x i32> <i32 3, i32 2, i32 1, i32 0>
  %84 = getelementptr inbounds i32, ptr %76, i64 -15
  store <4 x i32> %83, ptr %84, align 4, !tbaa !10
  %85 = add nuw i64 %68, 16
  %86 = add <4 x i32> %69, <i32 -16, i32 -16, i32 -16, i32 -16>
  %87 = icmp eq i64 %85, %62
  br i1 %87, label %88, label %67, !llvm.loop !17

88:                                               ; preds = %67
  %89 = icmp eq i64 %52, %62
  br i1 %89, label %99, label %90

90:                                               ; preds = %54, %50, %88
  %91 = phi i64 [ %51, %54 ], [ %51, %50 ], [ %63, %88 ]
  br label %92

92:                                               ; preds = %90, %92
  %93 = phi i64 [ %94, %92 ], [ %91, %90 ]
  %94 = add nsw i64 %93, -1
  %95 = and i64 %94, 4294967295
  %96 = getelementptr inbounds i32, ptr %12, i64 %95
  %97 = trunc i64 %93 to i32
  store i32 %97, ptr %96, align 4, !tbaa !10
  %98 = icmp ugt i64 %93, 2
  br i1 %98, label %92, label %99, !llvm.loop !18

99:                                               ; preds = %92, %88, %44
  tail call void @llvm.memcpy.p0.p0.i64(ptr noundef align 1 %10, ptr noundef align 1 %11, i64 noundef %9, i1 noundef false) #8
  %100 = load i32, ptr %10, align 4, !tbaa !10
  %101 = icmp eq i32 %100, 0
  br i1 %101, label %126, label %102

102:                                              ; preds = %99, %113
  %103 = phi i32 [ %114, %113 ], [ %100, %99 ]
  %104 = phi i32 [ %115, %113 ], [ 0, %99 ]
  %105 = icmp sgt i32 %103, 0
  br i1 %105, label %106, label %113

106:                                              ; preds = %102
  %107 = add nuw nsw i32 %103, 1
  %108 = lshr i32 %107, 1
  %109 = zext i32 %103 to i64
  %110 = zext i32 %108 to i64
  br label %117

111:                                              ; preds = %117
  %112 = load i32, ptr %10, align 4, !tbaa !10
  br label %113

113:                                              ; preds = %111, %102
  %114 = phi i32 [ %112, %111 ], [ %103, %102 ]
  %115 = add nuw nsw i32 %104, 1
  %116 = icmp eq i32 %114, 0
  br i1 %116, label %126, label %102, !llvm.loop !19

117:                                              ; preds = %106, %117
  %118 = phi i64 [ 0, %106 ], [ %124, %117 ]
  %119 = getelementptr inbounds i32, ptr %10, i64 %118
  %120 = load i32, ptr %119, align 4, !tbaa !10
  %121 = sub nsw i64 %109, %118
  %122 = getelementptr inbounds i32, ptr %10, i64 %121
  %123 = load i32, ptr %122, align 4, !tbaa !10
  store i32 %123, ptr %119, align 4, !tbaa !10
  store i32 %120, ptr %122, align 4, !tbaa !10
  %124 = add nuw nsw i64 %118, 1
  %125 = icmp eq i64 %124, %110
  br i1 %125, label %111, label %117, !llvm.loop !20

126:                                              ; preds = %113, %99
  %127 = phi i32 [ 0, %99 ], [ %115, %113 ]
  %128 = tail call i32 @llvm.smax.i32(i32 %127, i32 %48)
  %129 = and i32 %46, 1
  %130 = icmp eq i32 %129, 0
  %131 = sub nsw i32 0, %127
  %132 = select i1 %130, i32 %127, i32 %131
  %133 = add nsw i32 %132, %45
  %134 = add nuw nsw i32 %46, 1
  %135 = tail call i32 @llvm.smin.i32(i32 %47, i32 1)
  %136 = sext i32 %135 to i64
  br label %137

137:                                              ; preds = %147, %126
  %138 = phi i64 [ %153, %147 ], [ %136, %126 ]
  %139 = trunc i64 %138 to i32
  %140 = shl i64 %138, 2
  %141 = and i64 %140, 17179869180
  %142 = icmp eq i32 %7, %139
  br i1 %142, label %154, label %143

143:                                              ; preds = %137
  %144 = load i32, ptr %11, align 4, !tbaa !10
  %145 = icmp sgt i64 %138, 0
  br i1 %145, label %146, label %147

146:                                              ; preds = %143
  tail call void @llvm.memmove.p0.p0.i64(ptr nonnull align 4 %11, ptr align 4 %37, i64 %141, i1 false), !tbaa !10
  br label %147

147:                                              ; preds = %146, %143
  %148 = getelementptr inbounds i32, ptr %11, i64 %138
  store i32 %144, ptr %148, align 4, !tbaa !10
  %149 = getelementptr inbounds i32, ptr %12, i64 %138
  %150 = load i32, ptr %149, align 4, !tbaa !10
  %151 = add nsw i32 %150, -1
  store i32 %151, ptr %149, align 4, !tbaa !10
  %152 = icmp sgt i32 %150, 1
  %153 = add nsw i64 %138, 1
  br i1 %152, label %44, label %137

154:                                              ; preds = %137
  %155 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i32 noundef %133, i32 noundef %128)
  tail call void @free(ptr noundef %10)
  tail call void @free(ptr noundef %11)
  tail call void @free(ptr noundef %12)
  br label %156

156:                                              ; preds = %2, %154
  %157 = phi i32 [ 0, %154 ], [ 1, %2 ]
  ret i32 %157
}

; Function Attrs: mustprogress nofree nounwind willreturn memory(read)
declare i32 @atoi(ptr nocapture noundef) local_unnamed_addr #1

; Function Attrs: mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite)
declare noalias noundef ptr @malloc(i64 noundef) local_unnamed_addr #2

; Function Attrs: nofree nounwind
declare noundef i32 @printf(ptr nocapture noundef readonly, ...) local_unnamed_addr #3

; Function Attrs: mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite)
declare void @free(ptr allocptr nocapture noundef) local_unnamed_addr #4

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i32 @llvm.smax.i32(i32, i32) #5

; Function Attrs: nocallback nofree nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memmove.p0.p0.i64(ptr nocapture writeonly, ptr nocapture readonly, i64, i1 immarg) #6

; Function Attrs: nocallback nofree nosync nounwind speculatable willreturn memory(none)
declare i32 @llvm.smin.i32(i32, i32) #5

; Function Attrs: nocallback nofree nounwind willreturn memory(argmem: readwrite)
declare void @llvm.memcpy.p0.p0.i64(ptr noalias nocapture writeonly, ptr noalias nocapture readonly, i64, i1 immarg) #6

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #6 = { nocallback nofree nounwind willreturn memory(argmem: readwrite) }
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
!11 = !{!"int", !8, i64 0}
!12 = distinct !{!12, !13, !14, !15}
!13 = !{!"llvm.loop.mustprogress"}
!14 = !{!"llvm.loop.isvectorized", i32 1}
!15 = !{!"llvm.loop.unroll.runtime.disable"}
!16 = distinct !{!16, !13, !15, !14}
!17 = distinct !{!17, !13, !14, !15}
!18 = distinct !{!18, !13, !14}
!19 = distinct !{!19, !13}
!20 = distinct !{!20, !13}
