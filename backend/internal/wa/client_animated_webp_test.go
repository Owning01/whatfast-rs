package wa

import (
	"bytes"
	"encoding/binary"
	"image"
	"os"
	"path/filepath"
	"testing"
)

// bitWriter writes the least significant bit first, which is the bit order the
// WebP bitstream uses.
type bitWriter struct {
	data   []byte
	bitPos uint
}

func (w *bitWriter) write(count uint, value uint32) {
	for i := uint(0); i < count; i++ {
		if (value>>i)&1 == 0 {
			w.bitPos++
			continue
		}
		byteIndex := int(w.bitPos / 8)
		for byteIndex >= len(w.data) {
			w.data = append(w.data, 0)
		}
		w.data[byteIndex] |= 1 << (w.bitPos % 8)
		w.bitPos++
	}
}

func (w *bitWriter) bytes() []byte { return w.data }

// vp8lStill builds a minimal lossless VP8L bitstream that holds a single
// pixel. Every Huffman code uses the "simple" one symbol form, so the whole
// image is a handful of bits.
func vp8lStill() []byte {
	bits := &bitWriter{}
	bits.write(8, 0x2f) // signature
	bits.write(14, 0)   // width - 1
	bits.write(14, 0)   // height - 1
	bits.write(1, 0)    // alpha hint
	bits.write(3, 0)    // version
	bits.write(1, 0)    // no transform
	bits.write(1, 0)    // no color cache
	bits.write(1, 0)    // no meta Huffman group
	for i := 0; i < 5; i++ {
		bits.write(1, 1) // simple Huffman code
		bits.write(1, 0) // one symbol
		bits.write(1, 0) // one bit symbol
		bits.write(1, 0) // symbol zero
	}
	return bits.bytes()
}

func webpChunk(kind string, payload []byte) []byte {
	chunk := make([]byte, 0, 8+len(payload)+1)
	chunk = append(chunk, kind...)
	chunk = binary.LittleEndian.AppendUint32(chunk, uint32(len(payload)))
	chunk = append(chunk, payload...)
	if len(payload)%2 == 1 {
		chunk = append(chunk, 0)
	}
	return chunk
}

func webpFile(chunks ...[]byte) []byte {
	body := []byte("WEBP")
	for _, chunk := range chunks {
		body = append(body, chunk...)
	}
	file := []byte("RIFF")
	file = binary.LittleEndian.AppendUint32(file, uint32(len(body)))
	return append(file, body...)
}

// animatedWebP builds a real animated WebP file: a VP8X header with the
// animation flag, the global ANIM chunk and one ANMF frame that carries a
// complete VP8L still image. The standard WebP decoder ignores the animation
// chunks, so this file cannot be decoded as a whole.
func animatedWebP() []byte {
	vp8x := []byte{0x02, 0, 0, 0}
	vp8x = append(vp8x, 0, 0, 0)     // canvas width - 1
	vp8x = append(vp8x, 0, 0, 0)     // canvas height - 1
	anim := []byte{0, 0, 0, 0, 1, 0} // background colour and frame count
	frame := make([]byte, 0, 16)
	frame = append(frame, 0, 0, 0) // frame x
	frame = append(frame, 0, 0, 0) // frame y
	frame = append(frame, 0, 0, 0) // frame width - 1
	frame = append(frame, 0, 0, 0) // frame height - 1
	frame = append(frame, 0, 0, 0) // duration
	frame = append(frame, 0)       // flags
	frame = append(frame, webpChunk("VP8L", vp8lStill())...)
	return webpFile(webpChunk("VP8X", vp8x), webpChunk("ANIM", anim), webpChunk("ANMF", frame))
}

func TestAnimatedWebPFixtureIsRealAnimation(t *testing.T) {
	data := animatedWebP()
	if !webpIsAnimated(data) {
		t.Fatal("fixture was not detected as animated")
	}
	config, format, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil {
		t.Fatalf("decode config: %v", err)
	}
	if format != "webp" || config.Width != 1 || config.Height != 1 {
		t.Fatalf("format=%s dimensions=%dx%d", format, config.Width, config.Height)
	}
	if _, _, err = image.Decode(bytes.NewReader(data)); err == nil {
		t.Fatal("animated WebP decoded as a whole, the fixture is not an animation")
	}
}

func TestWebPFirstFrameLiftsTheAnimationFrame(t *testing.T) {
	frame, err := webpFirstFrame(animatedWebP())
	if err != nil {
		t.Fatal(err)
	}
	decoded, format, err := image.Decode(bytes.NewReader(frame))
	if err != nil {
		t.Fatalf("decode frame: %v", err)
	}
	if format != "webp" {
		t.Fatalf("frame format=%s", format)
	}
	if decoded.Bounds().Dx() != 1 || decoded.Bounds().Dy() != 1 {
		t.Fatalf("frame bounds=%v", decoded.Bounds())
	}
}

func TestAnimatedWebPThumbnailAndEmbeddedPreviewSucceed(t *testing.T) {
	data := animatedWebP()
	dir := t.TempDir()
	originalPath := filepath.Join(dir, "original.webp")
	if err := os.WriteFile(originalPath, data, 0o600); err != nil {
		t.Fatal(err)
	}
	thumbnailPath := filepath.Join(dir, "thumbnail.png")

	// The download path writes the thumbnail of an already cached original.
	c := &Client{mediaDir: dir}
	if err := c.writeThumbnail(originalPath, thumbnailPath); err != nil {
		t.Fatalf("write thumbnail: %v", err)
	}
	if _, err := os.Stat(thumbnailPath); err != nil {
		t.Fatalf("thumbnail was not published: %v", err)
	}
	thumbnail, format, err := image.DecodeConfig(bytes.NewReader(mustReadFile(t, thumbnailPath)))
	if err != nil {
		t.Fatalf("decode thumbnail: %v", err)
	}
	if format != "png" || thumbnail.Width != 1 || thumbnail.Height != 1 {
		t.Fatalf("thumbnail format=%s dimensions=%dx%d", format, thumbnail.Width, thumbnail.Height)
	}

	// The send path builds the embedded preview from the same thumbnail.
	embedded, err := embeddedJPEGThumbnail(thumbnailPath)
	if err != nil {
		t.Fatalf("embedded thumbnail: %v", err)
	}
	if len(embedded) == 0 {
		t.Fatal("embedded thumbnail is empty")
	}
	preview, format, err := image.DecodeConfig(bytes.NewReader(embedded))
	if err != nil {
		t.Fatalf("decode embedded thumbnail: %v", err)
	}
	if format != "jpeg" || preview.Width != 1 || preview.Height != 1 {
		t.Fatalf("embedded format=%s dimensions=%dx%d", format, preview.Width, preview.Height)
	}
}

func TestAnimatedWebPCacheKeepsOriginalAndThumbnail(t *testing.T) {
	data := animatedWebP()
	c := &Client{mediaDir: t.TempDir()}
	originalPath, thumbnailPath, err := c.cacheImageBytes("chat", "message", "image/webp", data)
	if err != nil {
		t.Fatalf("cache animated WebP: %v", err)
	}
	if originalPath == "" || thumbnailPath == "" {
		t.Fatalf("cache paths=(%q,%q)", originalPath, thumbnailPath)
	}
	cached, err := os.ReadFile(originalPath)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(cached, data) {
		t.Fatal("cached original does not match the uploaded bytes")
	}
	if _, err = os.Stat(thumbnailPath); err != nil {
		t.Fatalf("thumbnail was not published: %v", err)
	}
	if _, err = embeddedJPEGThumbnail(thumbnailPath); err != nil {
		t.Fatalf("embedded thumbnail: %v", err)
	}
}

func TestAnimatedWebPWithoutDecodableFrameStillCaches(t *testing.T) {
	// A frame whose image chunk is not a WebP bitstream cannot be decoded.
	// The cache must still publish the original instead of failing the send.
	data := webpFile(
		webpChunk("VP8X", []byte{0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0}),
		webpChunk("ANIM", []byte{0, 0, 0, 0, 1, 0}),
		webpChunk("ANMF", append(make([]byte, 16), webpChunk("VP8L", []byte("not a bitstream"))...)),
	)
	c := &Client{mediaDir: t.TempDir()}
	originalPath, thumbnailPath, err := c.cacheImageBytes("chat", "message", "image/webp", data)
	if err != nil {
		t.Fatalf("cache animated WebP: %v", err)
	}
	if originalPath == "" || thumbnailPath == "" {
		t.Fatalf("cache paths=(%q,%q)", originalPath, thumbnailPath)
	}
	embedded, err := embeddedJPEGThumbnail(thumbnailPath)
	if err != nil {
		t.Fatalf("embedded thumbnail: %v", err)
	}
	if len(embedded) != 0 {
		t.Fatal("a preview was invented for an undecodable frame")
	}
}

func mustReadFile(t *testing.T, path string) []byte {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return data
}
