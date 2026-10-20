package sample

import "strings"

// Client validates tokens.
type Client struct {
	Issuer string
}

type Verifier interface {
	Verify() bool
}

func NewClient(issuer string) *Client {
	return &Client{Issuer: issuer}
}

func (c *Client) Validate(token string) bool {
	return strings.TrimSpace(token) != ""
}
