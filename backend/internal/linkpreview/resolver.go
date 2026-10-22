// Package linkpreview resolves Open Graph metadata for links that arrive in a
// chat message without a preview attached.
//
// WhatsApp only ships a link preview when the sending client decided to build
// one, which happens rarely, so the desktop usually receives a bare URL. This
// resolver fetches the page itself and extracts the metadata a preview card
// needs. It is deliberately best effort: a page that cannot be parsed, is too
// large, or times out simply yields no preview and never fails the message it
// belongs to.
package linkpreview

import (
	"context"
	"fmt"
	"html"
	"io"
	"net"
	"net/http"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	// defaultTimeout bounds the whole request, including redirects and body
	// reads, so a slow host cannot pin a goroutine for long.
	defaultTimeout = 10 * time.Second
	// defaultTTL keeps a resolved page fresh for a while without refetching
	// the same link for every message that carries it.
	defaultTTL = 30 * time.Minute
	// defaultMaxEntries bounds the cache so a long-running session cannot
	// grow it without limit.
	defaultMaxEntries = 512
	// defaultMaxBody bounds how much HTML is read from a response.
	defaultMaxBody = 1 << 20
	// maxRedirects bounds the redirect chain a single request may follow.
	maxRedirects = 3
	// maxTitleLength and maxDescriptionLength bound the card text.
	maxTitleLength       = 200
	maxDescriptionLength = 400
	// defaultUserAgent is a regular desktop browser identity: link previews
	// are only rendered for pages that answer a normal browser request.
	defaultUserAgent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
)

// Preview is the metadata a link preview card is built from.
type Preview struct {
	URL         string
	Title       string
	Description string
	ImageURL    string
	SiteName    string
	Width       int
	Height      int
}

// Options tunes a Resolver. The zero value is valid and uses the defaults.
type Options struct {
	// HTTPClient overrides the HTTP client used for the fetch.
	HTTPClient *http.Client
	// UserAgent overrides the request user agent.
	UserAgent string
	// Timeout bounds a single request.
	Timeout time.Duration
	// TTL is how long a cached result stays valid.
	TTL time.Duration
	// MaxEntries bounds the number of cached URLs.
	MaxEntries int
	// MaxBody bounds the number of bytes read from a response body.
	MaxBody int64
	// BlockPrivateNetworks refuses loopback, private and link-local addresses.
	// A link that arrives in a chat message must not make the app touch the
	// machine it runs on or the network behind it.
	BlockPrivateNetworks bool
	// Now overrides the clock, which keeps the cache deterministic in tests.
	Now func() time.Time
}

type cacheEntry struct {
	preview   Preview
	found     bool
	expiresAt time.Time
}

// Resolver fetches and caches link previews. It is safe for concurrent use.
type Resolver struct {
	client     *http.Client
	userAgent  string
	ttl        time.Duration
	maxEntries int
	maxBody    int64
	now        func() time.Time

	mu    sync.Mutex
	cache map[string]cacheEntry
}

// New builds a Resolver from opts.
func New(opts Options) *Resolver {
	timeout := opts.Timeout
	if timeout <= 0 {
		timeout = defaultTimeout
	}
	userAgent := strings.TrimSpace(opts.UserAgent)
	if userAgent == "" {
		userAgent = defaultUserAgent
	}
	ttl := opts.TTL
	if ttl <= 0 {
		ttl = defaultTTL
	}
	maxEntries := opts.MaxEntries
	if maxEntries <= 0 {
		maxEntries = defaultMaxEntries
	}
	maxBody := opts.MaxBody
	if maxBody <= 0 {
		maxBody = defaultMaxBody
	}
	now := opts.Now
	if now == nil {
		now = time.Now
	}
	client := opts.HTTPClient
	if client == nil {
		transport := &http.Transport{}
		if opts.BlockPrivateNetworks {
			transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
				return dialPublic(ctx, network, address, timeout)
			}
		}
		client = &http.Client{
			Timeout:   timeout,
			Transport: transport,
			CheckRedirect: func(req *http.Request, via []*http.Request) error {
				if len(via) >= maxRedirects {
					return fmt.Errorf("too many redirects")
				}
				return nil
			},
		}
	}
	return &Resolver{
		client:     client,
		userAgent:  userAgent,
		ttl:        ttl,
		maxEntries: maxEntries,
		maxBody:    maxBody,
		now:        now,
		cache:      make(map[string]cacheEntry),
	}
}

// Resolve fetches the metadata for rawURL. The second return value reports
// whether a usable preview was found; a cached miss is reported the same way
// as a fresh miss so callers never retry the same dead link.
func (r *Resolver) Resolve(ctx context.Context, rawURL string) (Preview, bool) {
	target, ok := normalizeURL(rawURL)
	if !ok {
		return Preview{}, false
	}
	if preview, found, cached := r.cached(target); cached {
		return preview, found
	}
	preview, found := r.fetch(ctx, target)
	r.store(target, preview, found)
	return preview, found
}

// cached returns a cached entry when one is present and still valid.
func (r *Resolver) cached(target string) (Preview, bool, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	entry, ok := r.cache[target]
	if !ok || r.now().After(entry.expiresAt) {
		return Preview{}, false, false
	}
	return entry.preview, entry.found, true
}

// store records a result, evicting expired entries when the cache is full.
func (r *Resolver) store(target string, preview Preview, found bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if len(r.cache) >= r.maxEntries {
		now := r.now()
		for key, entry := range r.cache {
			if now.After(entry.expiresAt) {
				delete(r.cache, key)
			}
		}
	}
	if len(r.cache) >= r.maxEntries {
		// Every entry is still fresh: drop the one closest to expiry so the
		// cache stays bounded without discarding fresh results wholesale.
		oldest, oldestAt := "", time.Time{}
		for key, entry := range r.cache {
			if oldest == "" || entry.expiresAt.Before(oldestAt) {
				oldest, oldestAt = key, entry.expiresAt
			}
		}
		if oldest != "" {
			delete(r.cache, oldest)
		}
	}
	r.cache[target] = cacheEntry{preview: preview, found: found, expiresAt: r.now().Add(r.ttl)}
}

// fetch performs the request and parses the response body.
func (r *Resolver) fetch(ctx context.Context, target string) (Preview, bool) {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, target, nil)
	if err != nil {
		return Preview{}, false
	}
	request.Header.Set("User-Agent", r.userAgent)
	request.Header.Set("Accept", "text/html,application/xhtml+xml")
	response, err := r.client.Do(request)
	if err != nil {
		return Preview{}, false
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return Preview{}, false
	}
	contentType := strings.ToLower(response.Header.Get("Content-Type"))
	if contentType != "" && !strings.HasPrefix(contentType, "text/") && contentType != "application/xhtml+xml" {
		return Preview{}, false
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, r.maxBody))
	if err != nil || len(body) == 0 {
		return Preview{}, false
	}
	preview := parse(body, response.Request.URL)
	if preview.Title == "" && preview.Description == "" && preview.ImageURL == "" {
		return Preview{}, false
	}
	preview.URL = target
	return preview, true
}

// normalizeURL accepts absolute http and https URLs only.
func normalizeURL(rawURL string) (string, bool) {
	trimmed := strings.TrimSpace(rawURL)
	if trimmed == "" {
		return "", false
	}
	parsed, err := url.Parse(trimmed)
	if err != nil {
		return "", false
	}
	switch strings.ToLower(parsed.Scheme) {
	case "http", "https":
	default:
		return "", false
	}
	if parsed.Host == "" {
		return "", false
	}
	return parsed.String(), true
}

// carrierGradeNAT is the 100.64.0.0/10 block: not private by RFC 1918, but
// still not the public internet.
var carrierGradeNAT = func() *net.IPNet {
	_, block, _ := net.ParseCIDR("100.64.0.0/10")
	return block
}()

// dialPublic connects only to public addresses. It resolves the name first,
// refuses the request when any answer is private, and then dials the addresses
// it already checked: resolving twice would let the second answer point
// somewhere else.
func dialPublic(ctx context.Context, network, address string, timeout time.Duration) (net.Conn, error) {
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return nil, err
	}
	answers, err := net.DefaultResolver.LookupIPAddr(ctx, host)
	if err != nil {
		return nil, err
	}
	for _, answer := range answers {
		if !isPublic(answer.IP) {
			return nil, fmt.Errorf("linkpreview: %s no es una direccion publica", host)
		}
	}
	dialer := &net.Dialer{Timeout: timeout}
	var lastErr error
	for _, answer := range answers {
		conn, dialErr := dialer.DialContext(ctx, network, net.JoinHostPort(answer.IP.String(), port))
		if dialErr == nil {
			return conn, nil
		}
		lastErr = dialErr
	}
	if lastErr == nil {
		lastErr = fmt.Errorf("linkpreview: %s no resolvio a ninguna direccion", host)
	}
	return nil, lastErr
}

// isPublic reports whether ip belongs to the public internet.
func isPublic(ip net.IP) bool {
	if ip.IsLoopback() || ip.IsPrivate() || ip.IsUnspecified() ||
		ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() ||
		ip.IsMulticast() || ip.IsInterfaceLocalMulticast() {
		return false
	}
	return !carrierGradeNAT.Contains(ip)
}

var (
	metaTagPattern   = regexp.MustCompile(`(?is)<meta\b[^>]*>`)
	titlePattern     = regexp.MustCompile(`(?is)<title\b[^>]*>(.*?)</title>`)
	attributePattern = regexp.MustCompile("(?is)([a-zA-Z_:][-a-zA-Z0-9_:.]*)\\s*=\\s*(?:\"([^\"]*)\"|'([^']*)'|([^\\s\"'=<>`]+))")
)

// parse extracts the preview metadata from an HTML document. The scan is a
// best effort one: it understands the meta tags a preview card needs and
// ignores everything else, so a malformed document costs a missing preview
// instead of an error.
func parse(body []byte, base *url.URL) Preview {
	metadata := map[string]string{}
	for _, tag := range metaTagPattern.FindAll(body, -1) {
		attributes := parseAttributes(tag)
		key := strings.ToLower(strings.TrimSpace(attributes["property"]))
		if key == "" {
			key = strings.ToLower(strings.TrimSpace(attributes["name"]))
		}
		content, ok := attributes["content"]
		if key == "" || !ok {
			continue
		}
		key = strings.ToLower(strings.TrimSpace(key))
		if key == "" {
			continue
		}
		if _, exists := metadata[key]; !exists {
			metadata[key] = content
		}
	}
	preview := Preview{}
	preview.Title = cleanText(metadata["og:title"], maxTitleLength)
	if preview.Title == "" {
		if match := titlePattern.FindSubmatch(body); match != nil {
			preview.Title = cleanText(string(match[1]), maxTitleLength)
		}
	}
	preview.Description = cleanText(metadata["og:description"], maxDescriptionLength)
	if preview.Description == "" {
		preview.Description = cleanText(metadata["description"], maxDescriptionLength)
	}
	preview.SiteName = cleanText(metadata["og:site_name"], maxTitleLength)
	preview.ImageURL = absoluteURL(metadata["og:image"], base)
	preview.Width = parseDimension(metadata["og:image:width"])
	preview.Height = parseDimension(metadata["og:image:height"])
	return preview
}

// parseAttributes reads the attributes of a single tag into a lowercase map.
func parseAttributes(tag []byte) map[string]string {
	attributes := map[string]string{}
	for _, match := range attributePattern.FindAllSubmatch(tag, -1) {
		name := strings.ToLower(string(match[1]))
		value := string(match[2])
		if match[2] == nil {
			value = string(match[3])
		}
		if match[2] == nil && match[3] == nil {
			value = string(match[4])
		}
		if _, exists := attributes[name]; !exists {
			attributes[name] = value
		}
	}
	return attributes
}

// cleanText unescapes HTML entities, collapses whitespace and caps the length.
func cleanText(value string, limit int) string {
	unescaped := html.UnescapeString(value)
	collapsed := strings.Join(strings.Fields(unescaped), " ")
	if len(collapsed) <= limit {
		return collapsed
	}
	runes := []rune(collapsed)
	if len(runes) <= limit {
		return collapsed
	}
	return strings.TrimSpace(string(runes[:limit]))
}

// absoluteURL resolves a possibly relative image URL against the page URL.
func absoluteURL(value string, base *url.URL) string {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return ""
	}
	parsed, err := url.Parse(trimmed)
	if err != nil {
		return ""
	}
	if base != nil {
		parsed = base.ResolveReference(parsed)
	}
	return parsed.String()
}

// parseDimension reads an og:image dimension, ignoring malformed values.
func parseDimension(value string) int {
	parsed, err := strconv.Atoi(strings.TrimSpace(value))
	if err != nil || parsed < 0 {
		return 0
	}
	return parsed
}
