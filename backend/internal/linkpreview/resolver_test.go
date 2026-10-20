package linkpreview

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestResolveExtractsOpenGraphMetadata(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		requests.Add(1)
		writer.Header().Set("Content-Type", "text/html; charset=utf-8")
		fmt.Fprint(writer, `<!doctype html><html><head>
<meta content="A &amp; B" property="og:title">
<meta name='description' content='ignored because og wins'>
<meta property="og:description" content="Second &lt;line&gt;">
<meta property="og:image" content="/cover.png">
<meta property="og:image:width" content="640">
<meta property="og:image:height" content="480">
<meta property="og:site_name" content="Example News">
<title>Fallback title</title>
</head><body>ignored</body></html>`)
	}))
	defer server.Close()

	resolver := New(Options{})
	preview, found := resolver.Resolve(context.Background(), server.URL+"/story")
	if !found {
		t.Fatal("preview was not resolved")
	}
	if preview.Title != "A & B" {
		t.Fatalf("title=%q", preview.Title)
	}
	if preview.Description != "Second <line>" {
		t.Fatalf("description=%q", preview.Description)
	}
	if preview.ImageURL != server.URL+"/cover.png" {
		t.Fatalf("image=%q", preview.ImageURL)
	}
	if preview.Width != 640 || preview.Height != 480 {
		t.Fatalf("dimensions=%dx%d", preview.Width, preview.Height)
	}
	if preview.SiteName != "Example News" {
		t.Fatalf("site name=%q", preview.SiteName)
	}
	if preview.URL != server.URL+"/story" {
		t.Fatalf("url=%q", preview.URL)
	}
	if requests.Load() != 1 {
		t.Fatalf("requests=%d", requests.Load())
	}
}

func TestResolveFallsBackToTitleAndDescription(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprint(writer, `<html><head><title>  Plain   title  </title><meta name="description" content="Plain description"></head></html>`)
	}))
	defer server.Close()

	resolver := New(Options{})
	preview, found := resolver.Resolve(context.Background(), server.URL)
	if !found {
		t.Fatal("preview was not resolved")
	}
	if preview.Title != "Plain title" {
		t.Fatalf("title=%q", preview.Title)
	}
	if preview.Description != "Plain description" {
		t.Fatalf("description=%q", preview.Description)
	}
}

func TestResolveCapsLongMetadata(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprintf(writer, `<meta property="og:title" content="%s"><meta property="og:description" content="%s">`, strings.Repeat("t", 400), strings.Repeat("d", 900))
	}))
	defer server.Close()

	resolver := New(Options{})
	preview, found := resolver.Resolve(context.Background(), server.URL)
	if !found {
		t.Fatal("preview was not resolved")
	}
	if len([]rune(preview.Title)) != maxTitleLength {
		t.Fatalf("title length=%d", len([]rune(preview.Title)))
	}
	if len([]rune(preview.Description)) != maxDescriptionLength {
		t.Fatalf("description length=%d", len([]rune(preview.Description)))
	}
}

func TestResolveRejectsNonHTTPSchemes(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		requests.Add(1)
	}))
	defer server.Close()

	resolver := New(Options{})
	for _, rawURL := range []string{"ftp://example.com/file", "javascript:alert(1)", "file:///etc/passwd", "example.com/no-scheme", "   "} {
		if preview, found := resolver.Resolve(context.Background(), rawURL); found {
			t.Fatalf("%q was resolved: %+v", rawURL, preview)
		}
	}
	if requests.Load() != 0 {
		t.Fatalf("rejected URLs still reached the network: %d requests", requests.Load())
	}
}

func TestResolveTruncatesOversizedBody(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		requests.Add(1)
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprint(writer, `<meta property="og:title" content="Head title">`)
		// The rest of the document is far past the body limit.
		chunk := strings.Repeat("x", 64*1024)
		for i := 0; i < 64; i++ {
			if _, err := fmt.Fprint(writer, chunk); err != nil {
				return
			}
		}
	}))
	defer server.Close()

	resolver := New(Options{MaxBody: 4096})
	preview, found := resolver.Resolve(context.Background(), server.URL)
	if !found {
		t.Fatal("preview was not resolved")
	}
	if preview.Title != "Head title" {
		t.Fatalf("title=%q", preview.Title)
	}
	if requests.Load() != 1 {
		t.Fatalf("requests=%d", requests.Load())
	}
}

func TestResolveStopsReadingAtTheBodyLimit(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprintf(writer, "%s<meta property=\"og:title\" content=\"Tail title\">", strings.Repeat(" ", 4096))
	}))
	defer server.Close()

	// The metadata sits past the limit, so a resolver that honours the limit
	// never sees it.
	resolver := New(Options{MaxBody: 1024})
	if preview, found := resolver.Resolve(context.Background(), server.URL); found {
		t.Fatalf("metadata past the body limit was parsed: %+v", preview)
	}
}

func TestResolveHandlesBodyLargerThanTheDefaultLimit(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprint(writer, `<meta property="og:title" content="Big body">`)
		chunk := strings.Repeat("y", 64*1024)
		for i := 0; i < 48; i++ {
			if _, err := fmt.Fprint(writer, chunk); err != nil {
				return
			}
		}
	}))
	defer server.Close()

	resolver := New(Options{})
	preview, found := resolver.Resolve(context.Background(), server.URL)
	if !found {
		t.Fatal("preview was not resolved")
	}
	if preview.Title != "Big body" {
		t.Fatalf("title=%q", preview.Title)
	}
}

func TestResolveCachesSuccessfulAndFailedResults(t *testing.T) {
	var successful, failing atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path == "/missing" {
			failing.Add(1)
			writer.WriteHeader(http.StatusNotFound)
			return
		}
		successful.Add(1)
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprint(writer, `<meta property="og:title" content="Cached">`)
	}))
	defer server.Close()

	resolver := New(Options{})
	for i := 0; i < 3; i++ {
		preview, found := resolver.Resolve(context.Background(), server.URL+"/story")
		if !found || preview.Title != "Cached" {
			t.Fatalf("preview=%+v found=%v", preview, found)
		}
	}
	if successful.Load() != 1 {
		t.Fatalf("successful page was fetched %d times", successful.Load())
	}
	for i := 0; i < 3; i++ {
		if _, found := resolver.Resolve(context.Background(), server.URL+"/missing"); found {
			t.Fatal("missing page produced a preview")
		}
	}
	if failing.Load() != 1 {
		t.Fatalf("missing page was fetched %d times", failing.Load())
	}
}

func TestResolveExpiresCachedResults(t *testing.T) {
	var requests atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		requests.Add(1)
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprint(writer, `<meta property="og:title" content="Fresh">`)
	}))
	defer server.Close()

	now := time.Now()
	resolver := New(Options{TTL: time.Minute, Now: func() time.Time { return now }})
	if _, found := resolver.Resolve(context.Background(), server.URL); !found {
		t.Fatal("preview was not resolved")
	}
	now = now.Add(2 * time.Minute)
	if _, found := resolver.Resolve(context.Background(), server.URL); !found {
		t.Fatal("preview was not resolved after expiry")
	}
	if requests.Load() != 2 {
		t.Fatalf("requests=%d", requests.Load())
	}
}

func TestResolveSurvivesFailingHost(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {}))
	address := server.URL
	server.Close()

	resolver := New(Options{Timeout: 2 * time.Second})
	preview, found := resolver.Resolve(context.Background(), address)
	if found {
		t.Fatalf("dead host produced a preview: %+v", preview)
	}
}

func TestResolveIgnoresNonHTMLResponses(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "application/pdf")
		fmt.Fprint(writer, "%PDF-1.7")
	}))
	defer server.Close()

	resolver := New(Options{})
	if preview, found := resolver.Resolve(context.Background(), server.URL); found {
		t.Fatalf("binary response produced a preview: %+v", preview)
	}
}

func TestResolveBoundsTheCache(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprint(writer, `<meta property="og:title" content="Bounded">`)
	}))
	defer server.Close()

	resolver := New(Options{MaxEntries: 8})
	for i := 0; i < 32; i++ {
		if _, found := resolver.Resolve(context.Background(), fmt.Sprintf("%s/page-%d", server.URL, i)); !found {
			t.Fatalf("page %d was not resolved", i)
		}
	}
	resolver.mu.Lock()
	size := len(resolver.cache)
	resolver.mu.Unlock()
	if size > 8 {
		t.Fatalf("cache holds %d entries", size)
	}
}
