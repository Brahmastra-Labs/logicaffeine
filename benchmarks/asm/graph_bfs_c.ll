; ModuleID = '/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/graph_bfs/main.c'
source_filename = "/Users/tristen/logicaffeine/logicaffeine/benchmarks/programs/graph_bfs/main.c"
target datalayout = "e-m:o-i64:64-i128:128-n32:64-S128"
target triple = "arm64-apple-macosx15.0.0"

@.str = private unnamed_addr constant [9 x i8] c"%ld %ld\0A\00", align 1

; Function Attrs: nounwind ssp uwtable(sync)
define i32 @main(i32 noundef %0, ptr nocapture noundef readonly %1) local_unnamed_addr #0 {
  %3 = icmp slt i32 %0, 2
  br i1 %3, label %220, label %4

4:                                                ; preds = %2
  %5 = getelementptr inbounds ptr, ptr %1, i64 1
  %6 = load ptr, ptr %5, align 8, !tbaa !6
  %7 = tail call i64 @atol(ptr nocapture noundef %6)
  %8 = mul i64 %7, 40
  %9 = tail call ptr @malloc(i64 noundef %8) #8
  %10 = tail call ptr @calloc(i64 noundef %7, i64 noundef 4) #9
  %11 = icmp sgt i64 %7, 0
  br i1 %11, label %92, label %88

12:                                               ; preds = %107
  br i1 %11, label %13, label %88

13:                                               ; preds = %12, %28
  %14 = phi i64 [ %29, %28 ], [ 0, %12 ]
  %15 = mul nsw i64 %14, 37
  %16 = add nuw nsw i64 %15, 13
  %17 = srem i64 %16, %7
  %18 = icmp eq i64 %17, %14
  br i1 %18, label %28, label %19

19:                                               ; preds = %13
  %20 = getelementptr inbounds i32, ptr %10, i64 %14
  %21 = load i32, ptr %20, align 4, !tbaa !10
  %22 = trunc i64 %14 to i32
  %23 = mul i32 %22, 5
  %24 = add i32 %21, %23
  %25 = sext i32 %24 to i64
  %26 = getelementptr inbounds i64, ptr %9, i64 %25
  store i64 %17, ptr %26, align 8, !tbaa !12
  %27 = add nsw i32 %21, 1
  store i32 %27, ptr %20, align 4, !tbaa !10
  br label %28

28:                                               ; preds = %19, %13
  %29 = add nuw nsw i64 %14, 1
  %30 = icmp eq i64 %29, %7
  br i1 %30, label %31, label %13, !llvm.loop !14

31:                                               ; preds = %28
  br i1 %11, label %32, label %88

32:                                               ; preds = %31, %47
  %33 = phi i64 [ %48, %47 ], [ 0, %31 ]
  %34 = mul nsw i64 %33, 41
  %35 = add nuw nsw i64 %34, 17
  %36 = srem i64 %35, %7
  %37 = icmp eq i64 %36, %33
  br i1 %37, label %47, label %38

38:                                               ; preds = %32
  %39 = getelementptr inbounds i32, ptr %10, i64 %33
  %40 = load i32, ptr %39, align 4, !tbaa !10
  %41 = trunc i64 %33 to i32
  %42 = mul i32 %41, 5
  %43 = add i32 %40, %42
  %44 = sext i32 %43 to i64
  %45 = getelementptr inbounds i64, ptr %9, i64 %44
  store i64 %36, ptr %45, align 8, !tbaa !12
  %46 = add nsw i32 %40, 1
  store i32 %46, ptr %39, align 4, !tbaa !10
  br label %47

47:                                               ; preds = %38, %32
  %48 = add nuw nsw i64 %33, 1
  %49 = icmp eq i64 %48, %7
  br i1 %49, label %50, label %32, !llvm.loop !14

50:                                               ; preds = %47
  br i1 %11, label %51, label %88

51:                                               ; preds = %50, %66
  %52 = phi i64 [ %67, %66 ], [ 0, %50 ]
  %53 = mul nsw i64 %52, 43
  %54 = add nuw nsw i64 %53, 23
  %55 = srem i64 %54, %7
  %56 = icmp eq i64 %55, %52
  br i1 %56, label %66, label %57

57:                                               ; preds = %51
  %58 = getelementptr inbounds i32, ptr %10, i64 %52
  %59 = load i32, ptr %58, align 4, !tbaa !10
  %60 = trunc i64 %52 to i32
  %61 = mul i32 %60, 5
  %62 = add i32 %59, %61
  %63 = sext i32 %62 to i64
  %64 = getelementptr inbounds i64, ptr %9, i64 %63
  store i64 %55, ptr %64, align 8, !tbaa !12
  %65 = add nsw i32 %59, 1
  store i32 %65, ptr %58, align 4, !tbaa !10
  br label %66

66:                                               ; preds = %57, %51
  %67 = add nuw nsw i64 %52, 1
  %68 = icmp eq i64 %67, %7
  br i1 %68, label %69, label %51, !llvm.loop !14

69:                                               ; preds = %66
  br i1 %11, label %70, label %88

70:                                               ; preds = %69, %85
  %71 = phi i64 [ %86, %85 ], [ 0, %69 ]
  %72 = mul nsw i64 %71, 47
  %73 = add nuw nsw i64 %72, 29
  %74 = srem i64 %73, %7
  %75 = icmp eq i64 %74, %71
  br i1 %75, label %85, label %76

76:                                               ; preds = %70
  %77 = getelementptr inbounds i32, ptr %10, i64 %71
  %78 = load i32, ptr %77, align 4, !tbaa !10
  %79 = trunc i64 %71 to i32
  %80 = mul i32 %79, 5
  %81 = add i32 %78, %80
  %82 = sext i32 %81 to i64
  %83 = getelementptr inbounds i64, ptr %9, i64 %82
  store i64 %74, ptr %83, align 8, !tbaa !12
  %84 = add nsw i32 %78, 1
  store i32 %84, ptr %77, align 4, !tbaa !10
  br label %85

85:                                               ; preds = %76, %70
  %86 = add nuw nsw i64 %71, 1
  %87 = icmp eq i64 %86, %7
  br i1 %87, label %88, label %70, !llvm.loop !14

88:                                               ; preds = %85, %4, %12, %31, %50, %69
  %89 = shl i64 %7, 3
  %90 = tail call ptr @malloc(i64 noundef %89) #8
  %91 = tail call ptr @malloc(i64 noundef %89) #8
  tail call void @llvm.memset.p0.i64(ptr noundef align 1 %91, i8 noundef -1, i64 noundef %89, i1 noundef false) #10
  store i64 0, ptr %90, align 8, !tbaa !12
  store i64 0, ptr %91, align 8, !tbaa !12
  br label %172

92:                                               ; preds = %4, %107
  %93 = phi i64 [ %108, %107 ], [ 0, %4 ]
  %94 = mul nsw i64 %93, 31
  %95 = add nuw nsw i64 %94, 7
  %96 = srem i64 %95, %7
  %97 = icmp eq i64 %96, %93
  br i1 %97, label %107, label %98

98:                                               ; preds = %92
  %99 = getelementptr inbounds i32, ptr %10, i64 %93
  %100 = load i32, ptr %99, align 4, !tbaa !10
  %101 = trunc i64 %93 to i32
  %102 = mul i32 %101, 5
  %103 = add i32 %100, %102
  %104 = sext i32 %103 to i64
  %105 = getelementptr inbounds i64, ptr %9, i64 %104
  store i64 %96, ptr %105, align 8, !tbaa !12
  %106 = add nsw i32 %100, 1
  store i32 %106, ptr %99, align 4, !tbaa !10
  br label %107

107:                                              ; preds = %98, %92
  %108 = add nuw nsw i64 %93, 1
  %109 = icmp eq i64 %108, %7
  br i1 %109, label %12, label %92, !llvm.loop !14

110:                                              ; preds = %199, %172
  %111 = phi i64 [ %173, %172 ], [ %200, %199 ]
  %112 = icmp slt i64 %175, %111
  br i1 %112, label %172, label %113, !llvm.loop !16

113:                                              ; preds = %110
  br i1 %11, label %114, label %203

114:                                              ; preds = %113
  %115 = icmp ult i64 %7, 8
  br i1 %115, label %168, label %116

116:                                              ; preds = %114
  %117 = and i64 %7, -8
  br label %118

118:                                              ; preds = %118, %116
  %119 = phi i64 [ 0, %116 ], [ %156, %118 ]
  %120 = phi <2 x i64> [ zeroinitializer, %116 ], [ %152, %118 ]
  %121 = phi <2 x i64> [ zeroinitializer, %116 ], [ %153, %118 ]
  %122 = phi <2 x i64> [ zeroinitializer, %116 ], [ %154, %118 ]
  %123 = phi <2 x i64> [ zeroinitializer, %116 ], [ %155, %118 ]
  %124 = phi <2 x i64> [ zeroinitializer, %116 ], [ %144, %118 ]
  %125 = phi <2 x i64> [ zeroinitializer, %116 ], [ %145, %118 ]
  %126 = phi <2 x i64> [ zeroinitializer, %116 ], [ %146, %118 ]
  %127 = phi <2 x i64> [ zeroinitializer, %116 ], [ %147, %118 ]
  %128 = getelementptr inbounds i64, ptr %91, i64 %119
  %129 = load <2 x i64>, ptr %128, align 8, !tbaa !12
  %130 = getelementptr inbounds i64, ptr %128, i64 2
  %131 = load <2 x i64>, ptr %130, align 8, !tbaa !12
  %132 = getelementptr inbounds i64, ptr %128, i64 4
  %133 = load <2 x i64>, ptr %132, align 8, !tbaa !12
  %134 = getelementptr inbounds i64, ptr %128, i64 6
  %135 = load <2 x i64>, ptr %134, align 8, !tbaa !12
  %136 = icmp sgt <2 x i64> %129, <i64 -1, i64 -1>
  %137 = icmp sgt <2 x i64> %131, <i64 -1, i64 -1>
  %138 = icmp sgt <2 x i64> %133, <i64 -1, i64 -1>
  %139 = icmp sgt <2 x i64> %135, <i64 -1, i64 -1>
  %140 = zext <2 x i1> %136 to <2 x i64>
  %141 = zext <2 x i1> %137 to <2 x i64>
  %142 = zext <2 x i1> %138 to <2 x i64>
  %143 = zext <2 x i1> %139 to <2 x i64>
  %144 = add <2 x i64> %124, %140
  %145 = add <2 x i64> %125, %141
  %146 = add <2 x i64> %126, %142
  %147 = add <2 x i64> %127, %143
  %148 = select <2 x i1> %136, <2 x i64> %129, <2 x i64> zeroinitializer
  %149 = select <2 x i1> %137, <2 x i64> %131, <2 x i64> zeroinitializer
  %150 = select <2 x i1> %138, <2 x i64> %133, <2 x i64> zeroinitializer
  %151 = select <2 x i1> %139, <2 x i64> %135, <2 x i64> zeroinitializer
  %152 = add <2 x i64> %148, %120
  %153 = add <2 x i64> %149, %121
  %154 = add <2 x i64> %150, %122
  %155 = add <2 x i64> %151, %123
  %156 = add nuw i64 %119, 8
  %157 = icmp eq i64 %156, %117
  br i1 %157, label %158, label %118, !llvm.loop !17

158:                                              ; preds = %118
  %159 = add <2 x i64> %145, %144
  %160 = add <2 x i64> %146, %159
  %161 = add <2 x i64> %147, %160
  %162 = tail call i64 @llvm.vector.reduce.add.v2i64(<2 x i64> %161)
  %163 = add <2 x i64> %153, %152
  %164 = add <2 x i64> %154, %163
  %165 = add <2 x i64> %155, %164
  %166 = tail call i64 @llvm.vector.reduce.add.v2i64(<2 x i64> %165)
  %167 = icmp eq i64 %7, %117
  br i1 %167, label %203, label %168

168:                                              ; preds = %114, %158
  %169 = phi i64 [ 0, %114 ], [ %117, %158 ]
  %170 = phi i64 [ 0, %114 ], [ %166, %158 ]
  %171 = phi i64 [ 0, %114 ], [ %162, %158 ]
  br label %207

172:                                              ; preds = %88, %110
  %173 = phi i64 [ 1, %88 ], [ %111, %110 ]
  %174 = phi i64 [ 0, %88 ], [ %175, %110 ]
  %175 = add nuw nsw i64 %174, 1
  %176 = getelementptr inbounds i64, ptr %90, i64 %174
  %177 = load i64, ptr %176, align 8, !tbaa !12
  %178 = getelementptr inbounds i32, ptr %10, i64 %177
  %179 = load i32, ptr %178, align 4, !tbaa !10
  %180 = icmp sgt i32 %179, 0
  br i1 %180, label %181, label %110

181:                                              ; preds = %172
  %182 = mul nsw i64 %177, 5
  %183 = getelementptr inbounds i64, ptr %91, i64 %177
  %184 = zext i32 %179 to i64
  br label %185

185:                                              ; preds = %181, %199
  %186 = phi i64 [ 0, %181 ], [ %201, %199 ]
  %187 = phi i64 [ %173, %181 ], [ %200, %199 ]
  %188 = add nsw i64 %182, %186
  %189 = getelementptr inbounds i64, ptr %9, i64 %188
  %190 = load i64, ptr %189, align 8, !tbaa !12
  %191 = getelementptr inbounds i64, ptr %91, i64 %190
  %192 = load i64, ptr %191, align 8, !tbaa !12
  %193 = icmp eq i64 %192, -1
  br i1 %193, label %194, label %199

194:                                              ; preds = %185
  %195 = load i64, ptr %183, align 8, !tbaa !12
  %196 = add nsw i64 %195, 1
  store i64 %196, ptr %191, align 8, !tbaa !12
  %197 = add nsw i64 %187, 1
  %198 = getelementptr inbounds i64, ptr %90, i64 %187
  store i64 %190, ptr %198, align 8, !tbaa !12
  br label %199

199:                                              ; preds = %194, %185
  %200 = phi i64 [ %197, %194 ], [ %187, %185 ]
  %201 = add nuw nsw i64 %186, 1
  %202 = icmp eq i64 %201, %184
  br i1 %202, label %110, label %185, !llvm.loop !20

203:                                              ; preds = %207, %158, %113
  %204 = phi i64 [ 0, %113 ], [ %162, %158 ], [ %215, %207 ]
  %205 = phi i64 [ 0, %113 ], [ %166, %158 ], [ %217, %207 ]
  %206 = tail call i32 (ptr, ...) @printf(ptr noundef nonnull dereferenceable(1) @.str, i64 noundef %204, i64 noundef %205)
  tail call void @free(ptr noundef %9)
  tail call void @free(ptr noundef %10)
  tail call void @free(ptr noundef %90)
  tail call void @free(ptr noundef %91)
  br label %220

207:                                              ; preds = %168, %207
  %208 = phi i64 [ %218, %207 ], [ %169, %168 ]
  %209 = phi i64 [ %217, %207 ], [ %170, %168 ]
  %210 = phi i64 [ %215, %207 ], [ %171, %168 ]
  %211 = getelementptr inbounds i64, ptr %91, i64 %208
  %212 = load i64, ptr %211, align 8, !tbaa !12
  %213 = icmp sgt i64 %212, -1
  %214 = zext i1 %213 to i64
  %215 = add nuw nsw i64 %210, %214
  %216 = select i1 %213, i64 %212, i64 0
  %217 = add nsw i64 %216, %209
  %218 = add nuw nsw i64 %208, 1
  %219 = icmp eq i64 %218, %7
  br i1 %219, label %203, label %207, !llvm.loop !21

220:                                              ; preds = %2, %203
  %221 = phi i32 [ 0, %203 ], [ 1, %2 ]
  ret i32 %221
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
declare i64 @llvm.vector.reduce.add.v2i64(<2 x i64>) #6

; Function Attrs: nocallback nofree nounwind willreturn memory(argmem: write)
declare void @llvm.memset.p0.i64(ptr nocapture writeonly, i8, i64, i1 immarg) #7

attributes #0 = { nounwind ssp uwtable(sync) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #1 = { mustprogress nofree nounwind willreturn memory(read) "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #2 = { mustprogress nofree nounwind willreturn allockind("alloc,uninitialized") allocsize(0) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #3 = { mustprogress nofree nounwind willreturn allockind("alloc,zeroed") allocsize(0,1) memory(inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #4 = { nofree nounwind "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #5 = { mustprogress nounwind willreturn allockind("free") memory(argmem: readwrite, inaccessiblemem: readwrite) "alloc-family"="malloc" "frame-pointer"="non-leaf" "no-trapping-math"="true" "probe-stack"="__chkstk_darwin" "stack-protector-buffer-size"="8" "target-cpu"="apple-m1" "target-features"="+aes,+crc,+dotprod,+fp-armv8,+fp16fml,+fullfp16,+lse,+neon,+ras,+rcpc,+rdm,+sha2,+sha3,+v8.1a,+v8.2a,+v8.3a,+v8.4a,+v8.5a,+v8a,+zcm,+zcz" }
attributes #6 = { nocallback nofree nosync nounwind speculatable willreturn memory(none) }
attributes #7 = { nocallback nofree nounwind willreturn memory(argmem: write) }
attributes #8 = { allocsize(0) }
attributes #9 = { allocsize(0,1) }
attributes #10 = { nounwind }

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
!12 = !{!13, !13, i64 0}
!13 = !{!"long", !8, i64 0}
!14 = distinct !{!14, !15}
!15 = !{!"llvm.loop.mustprogress"}
!16 = distinct !{!16, !15}
!17 = distinct !{!17, !15, !18, !19}
!18 = !{!"llvm.loop.isvectorized", i32 1}
!19 = !{!"llvm.loop.unroll.runtime.disable"}
!20 = distinct !{!20, !15}
!21 = distinct !{!21, !15, !19, !18}
