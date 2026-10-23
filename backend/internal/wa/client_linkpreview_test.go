package wa

import (
	"context"
	"fmt"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"

	"github.com/Owning01/whatfast/backend/internal/domain"
	"github.com/Owning01/whatfast/backend/internal/linkpreview"
	"github.com/Owning01/whatfast/backend/internal/store"
)

func TestFirstLinkURLExtractsTheFirstHTTPURL(t *testing.T) {
	cases := []struct {
		text string
		want string
	}{
		{"look at https://example.com/story", "https://example.com/story"},
		{"http://example.com/a and https://example.com/b", "http://example.com/a"},
		{"no links here", ""},
		{"ftp://example.com/file", ""},
		{"see https://example.com/path.", "https://example.com/path"},
	}
	for _, testCase := range cases {
		if got := firstLinkURL(testCase.text); got != testCase.want {
			t.Fatalf("firstLinkURL(%q)=%q want %q", testCase.text, got, testCase.want)
		}
	}
}

func TestAttachLinkPreviewStoresTheResolvedPreview(t *testing.T) {
	ctx := context.Background()
	directory := t.TempDir()
	productStore, err := store.Open(ctx, filepath.Join(directory, "client.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer productStore.Close()
	events := make(chan Event, 16)
	client, err := New(ctx, directory, productStore, func(event Event) {
		select {
		case events <- event:
		default:
		}
	}, slog.Default())
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()

	// El servidor de prueba vive en 127.0.0.1 y el cliente real bloquea las
	// direcciones privadas: se le da un resolvedor sin ese bloqueo.
	client.linkPreviews = linkpreview.New(linkpreview.Options{})

	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprint(writer, `<meta property="og:title" content="Story title"><meta property="og:description" content="Story summary"><meta property="og:image" content="/cover.png">`)
	}))
	defer server.Close()

	chatID, _, err := productStore.EnsureConversation(ctx, "15551234567@s.whatsapp.net")
	if err != nil {
		t.Fatal(err)
	}
	message := domain.Message{
		ID: "message-1", ChatJID: chatID, TransportJID: "15551234567@s.whatsapp.net", SenderJID: "15551234567@s.whatsapp.net",
		Text: "look at " + server.URL + "/story", Timestamp: time.Now(), Kind: "text",
	}
	if err = productStore.ApplyMessage(ctx, message, false); err != nil {
		t.Fatal(err)
	}

	client.attachLinkPreview(message)

	deadline := time.Now().Add(15 * time.Second)
	var stored domain.Message
	for time.Now().Before(deadline) {
		stored, err = productStore.Message(ctx, chatID, "message-1")
		if err == nil && stored.LinkPreview != nil && stored.LinkPreview.Title != "" {
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	if stored.LinkPreview == nil {
		t.Fatal("the link preview was not stored")
	}
	if stored.LinkPreview.Title != "Story title" {
		t.Fatalf("title=%q", stored.LinkPreview.Title)
	}
	if stored.LinkPreview.Description == "" {
		t.Fatal("the description was not stored")
	}
	if stored.LinkPreview.URL != server.URL+"/story" {
		t.Fatalf("url=%q", stored.LinkPreview.URL)
	}
	if len(stored.LinkPreview.JPEGThumbnail) != 0 {
		t.Fatal("a thumbnail was invented without an image fetch")
	}
	select {
	case event := <-events:
		if event.Kind != "message" {
			t.Fatalf("event kind=%q", event.Kind)
		}
		if event.Message.LinkPreview == nil || event.Message.LinkPreview.Title != "Story title" {
			t.Fatalf("event preview=%+v", event.Message.LinkPreview)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the desktop was not notified about the preview")
	}
}

// A page can answer after the user logged out. The preview belongs to the
// account that left and must not recreate its message in the cleared store.
func TestAttachLinkPreviewDoesNotWriteAfterTheAccountIsIsolated(t *testing.T) {
	ctx := context.Background()
	directory := t.TempDir()
	productStore, err := store.Open(ctx, filepath.Join(directory, "client.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer productStore.Close()
	client, err := New(ctx, directory, productStore, func(Event) {}, slog.Default())
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	client.linkPreviews = linkpreview.New(linkpreview.Options{})

	started := make(chan struct{})
	release := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		close(started)
		<-release
		writer.Header().Set("Content-Type", "text/html")
		fmt.Fprint(writer, `<meta property="og:title" content="Old account story">`)
	}))
	defer server.Close()

	chatID, _, err := productStore.EnsureConversation(ctx, "15551234567@s.whatsapp.net")
	if err != nil {
		t.Fatal(err)
	}
	message := domain.Message{
		ID: "message-1", ChatJID: chatID, TransportJID: "15551234567@s.whatsapp.net", SenderJID: "15551234567@s.whatsapp.net",
		Text: "look at " + server.URL + "/story", Timestamp: time.Now(), Kind: "text",
	}
	if err = productStore.ApplyMessage(ctx, message, false); err != nil {
		t.Fatal(err)
	}
	client.attachLinkPreview(message)
	select {
	case <-started:
	case <-time.After(5 * time.Second):
		t.Fatal("the preview was never fetched")
	}
	// La cuenta se cierra mientras la pagina sigue sin contestar: la respuesta
	// llega despues del cambio de generacion.
	client.accepting.Store(false)
	close(release)
	client.Close()

	stored, err := productStore.Message(ctx, chatID, message.ID)
	if err != nil {
		t.Fatal(err)
	}
	if stored.LinkPreview != nil {
		t.Fatalf("the preview of the previous account came back: %+v", stored.LinkPreview)
	}
}

func TestAttachLinkPreviewSkipsMessagesWithoutLinks(t *testing.T) {
	ctx := context.Background()
	directory := t.TempDir()
	productStore, err := store.Open(ctx, filepath.Join(directory, "client.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer productStore.Close()
	client, err := New(ctx, directory, productStore, func(Event) {}, slog.Default())
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()

	chatID, _, err := productStore.EnsureConversation(ctx, "15551234567@s.whatsapp.net")
	if err != nil {
		t.Fatal(err)
	}
	message := domain.Message{
		ID: "message-1", ChatJID: chatID, TransportJID: "15551234567@s.whatsapp.net", SenderJID: "15551234567@s.whatsapp.net",
		Text: "no link here", Timestamp: time.Now(), Kind: "text",
	}
	if err = productStore.ApplyMessage(ctx, message, false); err != nil {
		t.Fatal(err)
	}
	client.attachLinkPreview(message)
	time.Sleep(200 * time.Millisecond)
	stored, err := productStore.Message(ctx, chatID, "message-1")
	if err != nil {
		t.Fatal(err)
	}
	if stored.LinkPreview != nil {
		t.Fatalf("preview=%+v", stored.LinkPreview)
	}
}
