package store

import (
	"context"
	"path/filepath"
	"testing"
	"time"

	"github.com/Owning01/whatfast/backend/internal/domain"
)

func TestSetChatBlockedRoundTripsAndSurvivesReopen(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "client.db")
	s, err := Open(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	chatID := "15551234567@s.whatsapp.net"

	// Blocking a chat that has no conversation yet creates it, so the block
	// survives the first message the contact sends.
	if err = s.SetChatBlocked(ctx, chatID, true); err != nil {
		t.Fatal(err)
	}
	chat, err := s.Chat(ctx, chatID)
	if err != nil {
		t.Fatal(err)
	}
	if !chat.Blocked {
		t.Fatal("chat is not blocked")
	}
	page, err := s.Chats(ctx, "", 50)
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, item := range page.Items {
		if item.JID == chat.JID {
			found = true
			if !item.Blocked {
				t.Fatal("chat list does not carry the blocked flag")
			}
		}
	}
	if !found {
		t.Fatal("blocked chat is missing from the chat list")
	}

	// A message does not clear the block.
	if err = s.ApplyMessage(ctx, domain.Message{
		ID: "message-1", ChatJID: chatID, TransportJID: chatID, SenderJID: chatID,
		Text: "hello", Timestamp: time.UnixMilli(1000), Kind: "text",
	}, true); err != nil {
		t.Fatal(err)
	}
	chat, err = s.Chat(ctx, chatID)
	if err != nil {
		t.Fatal(err)
	}
	if !chat.Blocked {
		t.Fatal("the first message cleared the block")
	}

	if err = s.Close(); err != nil {
		t.Fatal(err)
	}
	s, err = Open(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	chat, err = s.Chat(ctx, chatID)
	if err != nil {
		t.Fatal(err)
	}
	if !chat.Blocked {
		t.Fatal("the block did not survive the reopen")
	}

	if err = s.SetChatBlocked(ctx, chatID, false); err != nil {
		t.Fatal(err)
	}
	chat, err = s.Chat(ctx, chatID)
	if err != nil {
		t.Fatal(err)
	}
	if chat.Blocked {
		t.Fatal("the chat is still blocked")
	}
}

func TestChatBlockedDefaultsToFalse(t *testing.T) {
	ctx := context.Background()
	s, err := Open(ctx, filepath.Join(t.TempDir(), "client.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	chatID := "15559998877@s.whatsapp.net"
	if err = s.ApplyMessage(ctx, domain.Message{
		ID: "message-1", ChatJID: chatID, TransportJID: chatID, SenderJID: chatID,
		Text: "hello", Timestamp: time.UnixMilli(1000), Kind: "text",
	}, true); err != nil {
		t.Fatal(err)
	}
	chat, err := s.Chat(ctx, chatID)
	if err != nil {
		t.Fatal(err)
	}
	if chat.Blocked {
		t.Fatal("a chat is blocked without an explicit block")
	}
}
