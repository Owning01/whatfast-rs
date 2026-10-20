package wa

import (
	"bytes"
	"encoding/binary"
	"image"
	"image/color"
	"image/gif"
	"os"
	"path/filepath"
	"testing"

	"github.com/Owning01/whatfast/backend/internal/domain"
	"go.mau.fi/whatsmeow"
)

func isoTestBox(kind string, payload []byte) []byte {
	box := make([]byte, 0, 8+len(payload))
	box = binary.BigEndian.AppendUint32(box, uint32(8+len(payload)))
	box = append(box, kind...)
	return append(box, payload...)
}

// minimalMP4 builds an ISO base media file with a movie header that claims a
// duration of 25 seconds and a display size of 1280x720.
func minimalMP4() []byte {
	ftyp := isoTestBox("ftyp", []byte("isom\x00\x00\x02\x00isom"))
	mvhd := make([]byte, 100)
	binary.BigEndian.PutUint32(mvhd[12:16], 1000)  // timescale
	binary.BigEndian.PutUint32(mvhd[16:20], 25000) // duration
	mvhdBox := isoTestBox("mvhd", mvhd)
	tkhd := make([]byte, 84)
	binary.BigEndian.PutUint32(tkhd[len(tkhd)-8:len(tkhd)-4], 1280<<16)
	binary.BigEndian.PutUint32(tkhd[len(tkhd)-4:], 720<<16)
	trak := isoTestBox("trak", isoTestBox("tkhd", tkhd))
	moov := isoTestBox("moov", append(mvhdBox, trak...))
	return append(ftyp, moov...)
}

// riffChunk writes a RIFF chunk, whose size is little endian.
func riffChunk(kind string, payload []byte) []byte {
	chunk := make([]byte, 0, 8+len(payload))
	chunk = append(chunk, kind...)
	chunk = binary.LittleEndian.AppendUint32(chunk, uint32(len(payload)))
	return append(chunk, payload...)
}

// minimalOgg builds an Ogg stream whose last page carries a granule position
// of three seconds of 48 kHz samples.
func minimalOgg() []byte {
	opusHead := append([]byte("OpusHead"), 1, 2, 0, 0, 0x80, 0xBB, 0x00, 0x00, 0, 0, 0)
	page := func(granule uint64, sequence uint32, payload []byte) []byte {
		header := []byte("OggS")
		header = append(header, 0, 2)
		header = binary.LittleEndian.AppendUint64(header, granule)
		header = binary.LittleEndian.AppendUint32(header, 1) // serial
		header = binary.LittleEndian.AppendUint32(header, sequence)
		header = binary.LittleEndian.AppendUint32(header, 0) // checksum
		header = append(header, 1)                           // segment count
		header = append(header, byte(len(payload)))          // segment table
		return append(header, payload...)
	}
	first := page(0, 0, opusHead)
	second := page(48000*3, 1, []byte{0x01, 0x02, 0x03})
	return append(first, second...)
}

// minimalWAV builds a RIFF WAVE file with one second of 8 bit mono audio.
func minimalWAV() []byte {
	fmtChunk := make([]byte, 16)
	binary.LittleEndian.PutUint16(fmtChunk[0:2], 1)     // PCM
	binary.LittleEndian.PutUint16(fmtChunk[2:4], 1)     // channels
	binary.LittleEndian.PutUint32(fmtChunk[4:8], 8000)  // sample rate
	binary.LittleEndian.PutUint32(fmtChunk[8:12], 8000) // byte rate
	binary.LittleEndian.PutUint16(fmtChunk[12:14], 1)   // block align
	binary.LittleEndian.PutUint16(fmtChunk[14:16], 8)   // bits per sample
	data := make([]byte, 8000)
	body := []byte("WAVE")
	body = append(body, riffChunk("fmt ", fmtChunk)...)
	body = append(body, riffChunk("data", data)...)
	file := []byte("RIFF")
	file = binary.LittleEndian.AppendUint32(file, uint32(len(body)))
	return append(file, body...)
}

func animatedGIF(t *testing.T) []byte {
	t.Helper()
	palette := color.Palette{color.Black, color.White}
	first := image.NewPaletted(image.Rect(0, 0, 4, 4), palette)
	second := image.NewPaletted(image.Rect(0, 0, 4, 4), palette)
	for x := 0; x < 4; x++ {
		for y := 0; y < 4; y++ {
			first.SetColorIndex(x, y, 0)
			second.SetColorIndex(x, y, 1)
		}
	}
	var encoded bytes.Buffer
	if err := gif.EncodeAll(&encoded, &gif.GIF{Image: []*image.Paletted{first, second}, Delay: []int{10, 10}, LoopCount: 0}); err != nil {
		t.Fatal(err)
	}
	return encoded.Bytes()
}

func writeMediaFixture(t *testing.T, name string, data []byte) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), name)
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestProbeOutgoingMediaReadsMP4Metadata(t *testing.T) {
	path := writeMediaFixture(t, "clip.mp4", minimalMP4())
	metadata := probeOutgoingMedia(path)
	if metadata.durationSeconds != 25 {
		t.Fatalf("duration=%d", metadata.durationSeconds)
	}
	if metadata.width != 1280 || metadata.height != 720 {
		t.Fatalf("dimensions=%dx%d", metadata.width, metadata.height)
	}
}

func TestProbeOutgoingMediaReadsOggDuration(t *testing.T) {
	path := writeMediaFixture(t, "voice.ogg", minimalOgg())
	metadata := probeOutgoingMedia(path)
	if metadata.durationSeconds != 3 {
		t.Fatalf("duration=%d", metadata.durationSeconds)
	}
}

func TestProbeOutgoingMediaReadsWAVDuration(t *testing.T) {
	path := writeMediaFixture(t, "clip.wav", minimalWAV())
	metadata := probeOutgoingMedia(path)
	if metadata.durationSeconds != 1 {
		t.Fatalf("duration=%d", metadata.durationSeconds)
	}
}

func TestProbeOutgoingMediaDetectsAnimatedGIF(t *testing.T) {
	path := writeMediaFixture(t, "loop.gif", animatedGIF(t))
	metadata := probeOutgoingMedia(path)
	if !metadata.animated {
		t.Fatal("animated GIF was not detected")
	}
	if metadata.width != 4 || metadata.height != 4 {
		t.Fatalf("dimensions=%dx%d", metadata.width, metadata.height)
	}
}

func TestProbeOutgoingMediaIgnoresUnknownFormats(t *testing.T) {
	path := writeMediaFixture(t, "notes.txt", []byte("plain text"))
	metadata := probeOutgoingMedia(path)
	if metadata.durationSeconds != 0 || metadata.width != 0 || metadata.height != 0 || metadata.animated {
		t.Fatalf("metadata=%+v", metadata)
	}
}

func TestOutgoingAttachmentMessageCarriesMediaMetadata(t *testing.T) {
	upload := whatsmeow.UploadResponse{
		URL: "https://media.example/file", DirectPath: "/mms/file", FileLength: 123,
		MediaKey: bytes.Repeat([]byte{1}, 32), FileSHA256: bytes.Repeat([]byte{2}, 32), FileEncSHA256: bytes.Repeat([]byte{3}, 32),
	}
	video, err := outgoingAttachmentMessage("video", &domain.Attachment{
		Caption: "clip", MIMEType: "video/mp4", FileName: "clip.mp4", DurationSeconds: 12, Width: 1280, Height: 720,
	}, upload, nil)
	if err != nil {
		t.Fatal(err)
	}
	wire := video.GetVideoMessage()
	if wire.GetSeconds() != 12 {
		t.Fatalf("seconds=%d", wire.GetSeconds())
	}
	if wire.GetWidth() != 1280 || wire.GetHeight() != 720 {
		t.Fatalf("dimensions=%dx%d", wire.GetWidth(), wire.GetHeight())
	}
	if wire.GetGifPlayback() {
		t.Fatal("a still video was marked as a GIF")
	}
	audio, err := outgoingAttachmentMessage("audio", &domain.Attachment{
		MIMEType: "audio/ogg; codecs=opus", FileName: "voice.ogg", DurationSeconds: 7, VoiceNote: true,
	}, upload, nil)
	if err != nil {
		t.Fatal(err)
	}
	if audio.GetAudioMessage().GetSeconds() != 7 {
		t.Fatalf("seconds=%d", audio.GetAudioMessage().GetSeconds())
	}
	if !audio.GetAudioMessage().GetPTT() {
		t.Fatal("voice note flag was lost")
	}
	gif, err := outgoingAttachmentMessage("video", &domain.Attachment{
		MIMEType: "video/mp4", FileName: "loop.gif", DurationSeconds: 2, Width: 320, Height: 240, Animated: true,
	}, upload, nil)
	if err != nil {
		t.Fatal(err)
	}
	if !gif.GetVideoMessage().GetGifPlayback() {
		t.Fatal("animated attachment was not marked as a GIF")
	}
	if gif.GetVideoMessage().GetSeconds() != 2 {
		t.Fatalf("seconds=%d", gif.GetVideoMessage().GetSeconds())
	}
}

func TestOutgoingImageAnimatedFlagFollowsTheFile(t *testing.T) {
	data := animatedGIF(t)
	if !outgoingImageAnimated("image/gif", data) {
		t.Fatal("animated GIF was not detected")
	}
	if outgoingImageAnimated("image/gif", data[:20]) {
		t.Fatal("a truncated GIF was reported as animated")
	}
	if outgoingImageAnimated("image/png", data) {
		t.Fatal("a PNG was reported as animated")
	}
	if !outgoingImageAnimated("image/webp", animatedWebP()) {
		t.Fatal("animated WebP was not detected")
	}
}
