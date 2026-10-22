Tiny MP4s for `ptv_core::media` tests, made with the vendored ffmpeg:

```bash
common="-f lavfi -i testsrc2=size=64x36:rate=30:duration=0.5 -f lavfi -i sine=frequency=440:duration=0.5 -f lavfi -i sine=frequency=880:duration=0.5 -map 0:v -map 1:a -map 2:a -c:a aac -b:a 32k -metadata:s:a:0 handler_name=Mixed -metadata:s:a:0 title=Mixed -metadata:s:a:1 handler_name=Mic -metadata:s:a:1 title=Mic"
ffmpeg $common -c:v libx264 -pix_fmt yuv420p -movflags +faststart h264.mp4
ffmpeg $common -c:v libx264 -pix_fmt yuv444p h264_444_moov_at_end.mp4
ffmpeg $common -c:v libx265 -pix_fmt yuv420p -tag:v hvc1 -movflags +faststart hevc.mp4
ffmpeg $common -c:v libaom-av1 -cpu-used 8 -pix_fmt yuv420p -movflags +faststart av1.mp4
ffmpeg -f lavfi -i testsrc2=size=64x36:rate=30:duration=0.2 -c:v libx265 -pix_fmt yuv444p -tag:v hvc1 hevc_444.mp4
```
