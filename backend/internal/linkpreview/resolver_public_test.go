package linkpreview

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"testing"
)

// Un enlace que llega en un mensaje no puede hacer que la app pida algo de la
// propia maquina ni de la red local.
func TestResolverBlocksPrivateNetworks(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html; charset=utf-8")
		fmt.Fprint(writer, `<!doctype html><html><head><title>No deberia leerse</title></head></html>`)
	}))
	defer server.Close()

	resolver := New(Options{BlockPrivateNetworks: true})
	preview, found := resolver.Resolve(context.Background(), server.URL+"/privado")
	if found {
		t.Fatalf("se leyo una direccion privada: %+v", preview)
	}
}

// Con la opcion apagada, el mismo servidor de prueba se lee: asi queda claro
// que el bloqueo es la opcion y no un efecto del entorno de test.
func TestResolverReadsLoopbackWithoutTheOption(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/html; charset=utf-8")
		fmt.Fprint(writer, `<!doctype html><html><head><title>Servidor de prueba</title></head></html>`)
	}))
	defer server.Close()

	resolver := New(Options{})
	preview, found := resolver.Resolve(context.Background(), server.URL+"/local")
	if !found {
		t.Fatal("sin la opcion, la direccion local deberia leerse")
	}
	if preview.Title != "Servidor de prueba" {
		t.Fatalf("title=%q", preview.Title)
	}
}

func TestIsPublic(t *testing.T) {
	casos := []struct {
		ip     string
		public bool
	}{
		{"127.0.0.1", false},
		{"127.1.2.3", false},
		{"0.0.0.0", false},
		{"10.0.0.5", false},
		{"172.16.3.4", false},
		{"192.168.1.10", false},
		{"169.254.169.254", false},
		{"100.64.0.7", false},
		{"224.0.0.1", false},
		{"::1", false},
		{"fc00::1", false},
		{"fe80::1", false},
		{"8.8.8.8", true},
		{"1.1.1.1", true},
		{"2606:4700:4700::1111", true},
	}
	for _, caso := range casos {
		ip := net.ParseIP(caso.ip)
		if ip == nil {
			t.Fatalf("ip invalida en el caso: %s", caso.ip)
		}
		if got := isPublic(ip); got != caso.public {
			t.Fatalf("isPublic(%s)=%v, se esperaba %v", caso.ip, got, caso.public)
		}
	}
}
