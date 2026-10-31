package infra

import (
	"context"
	"errors"
	"net/http"
	"testing"
	"time"

	"github.com/descope/go-sdk/descope"
)

const (
	rateLimitMsg = "rate limited"
)

func newRateLimitError(retryAfter ...int) *descope.Error {
	err := descope.ErrRateLimitExceeded.WithMessage(rateLimitMsg)
	if len(retryAfter) > 0 {
		err = err.WithInfo(descope.ErrorInfoKeys.RateLimitExceededRetryAfter, retryAfter[0])
	}
	return err
}

func newServerError(statusCode int) *descope.Error {
	return (&descope.Error{Code: "E999999", Message: "server error"}).
		WithInfo(descope.ErrorInfoKeys.HTTPResponseStatusCode, statusCode)
}

func requireNoError(t *testing.T, err error) {
	t.Helper()
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
}

func requireCallCount(t *testing.T, got, want int) {
	t.Helper()
	if got != want {
		t.Fatalf("expected %d call(s), got %d", want, got)
	}
}

func TestRetryOnRateLimit_Success(t *testing.T) {
	calls := 0
	result, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		return "ok", nil
	})
	requireNoError(t, err)
	if result != "ok" {
		t.Fatalf("expected 'ok', got %q", result)
	}
	requireCallCount(t, calls, 1)
}

func TestRetryOnRateLimit_NonRateLimitError(t *testing.T) {
	calls := 0
	_, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		return "", errors.New("some other error")
	})
	if err == nil || err.Error() != "some other error" {
		t.Fatalf("expected 'some other error', got %v", err)
	}
	requireCallCount(t, calls, 1)
}

func TestRetryOnRateLimit_ContextCancelled(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	calls := 0
	_, err := RetryOnRateLimit(ctx, func() (string, error) {
		calls++
		return "", newRateLimitError()
	})
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("expected context.Canceled, got %v", err)
	}
}

func TestRetryOnRateLimit_NonRateLimitDescopeError(t *testing.T) {
	calls := 0
	descopeErr := &descope.Error{Code: "E999999", Message: "bad request"}
	_, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		return "", descopeErr
	})
	if err == nil {
		t.Fatal("expected error")
	}
	requireCallCount(t, calls, 1)
}

func TestRetryOnRateLimit_RetriesThenSucceeds(t *testing.T) {
	calls := 0
	start := time.Now()
	result, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		if calls <= 2 {
			return "", newRateLimitError(1)
		}
		return "recovered", nil
	})
	elapsed := time.Since(start)
	requireNoError(t, err)
	if result != "recovered" {
		t.Fatalf("expected 'recovered', got %q", result)
	}
	requireCallCount(t, calls, 3)
	if elapsed < 1*time.Second {
		t.Fatalf("expected at least 1s of retry wait, got %v", elapsed)
	}
}

func TestRetryOnRateLimit_ExhaustsRetries(t *testing.T) {
	calls := 0
	_, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		return "", newRateLimitError(1)
	})
	if err == nil {
		t.Fatal("expected error after exhausting retries")
	}
	// maxRetries (3) loop iterations + 1 final call = 4 total
	requireCallCount(t, calls, maxRetries+1)
}

func TestRetryOnRateLimit_UsesRetryAfter(t *testing.T) {
	calls := 0
	start := time.Now()
	_, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		if calls == 1 {
			return "", newRateLimitError(2)
		}
		return "ok", nil
	})
	elapsed := time.Since(start)
	requireNoError(t, err)
	if elapsed < 2*time.Second {
		t.Fatalf("expected at least 2s retry wait from Retry-After, got %v", elapsed)
	}
	if elapsed > 5*time.Second {
		t.Fatalf("waited too long (%v), Retry-After may not be respected", elapsed)
	}
}

func TestRetryOnRateLimit_DefaultWaitFallback(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	calls := 0
	_, err := RetryOnRateLimit(ctx, func() (string, error) {
		calls++
		return "", newRateLimitError()
	})
	if !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("expected context.DeadlineExceeded (default wait > context timeout), got %v", err)
	}
	requireCallCount(t, calls, 1)
}

func TestRetryOnRateLimitNoResult_Success(t *testing.T) {
	calls := 0
	err := RetryOnRateLimitNoResult(context.Background(), func() error {
		calls++
		return nil
	})
	requireNoError(t, err)
	requireCallCount(t, calls, 1)
}

func TestRetryOnRateLimitNoResult_Error(t *testing.T) {
	err := RetryOnRateLimitNoResult(context.Background(), func() error {
		return errors.New("failed")
	})
	if err == nil || err.Error() != "failed" {
		t.Fatalf("expected 'failed', got %v", err)
	}
}

func TestRetryOnRateLimitNoResult_RetriesThenSucceeds(t *testing.T) {
	calls := 0
	err := RetryOnRateLimitNoResult(context.Background(), func() error {
		calls++
		if calls == 1 {
			return newRateLimitError(1)
		}
		return nil
	})
	requireNoError(t, err)
	requireCallCount(t, calls, 2)
}

// Tests for transient server error (5xx) retry behavior

func TestRetryOnRateLimit_ServerError500_RetriesThenSucceeds(t *testing.T) {
	calls := 0
	start := time.Now()
	result, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		if calls == 1 {
			return "", newServerError(http.StatusInternalServerError)
		}
		return "recovered", nil
	})
	elapsed := time.Since(start)
	requireNoError(t, err)
	if result != "recovered" {
		t.Fatalf("expected 'recovered', got %q", result)
	}
	requireCallCount(t, calls, 2)
	if elapsed < transientRetryBaseWait {
		t.Fatalf("expected at least %v of retry wait, got %v", transientRetryBaseWait, elapsed)
	}
}

func TestRetryOnRateLimit_ServerError502_RetriesThenSucceeds(t *testing.T) {
	calls := 0
	result, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		if calls == 1 {
			return "", newServerError(http.StatusBadGateway)
		}
		return "ok", nil
	})
	requireNoError(t, err)
	if result != "ok" {
		t.Fatalf("expected 'ok', got %q", result)
	}
	requireCallCount(t, calls, 2)
}

func TestRetryOnRateLimit_ServerError503_RetriesThenSucceeds(t *testing.T) {
	calls := 0
	result, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		if calls == 1 {
			return "", newServerError(http.StatusServiceUnavailable)
		}
		return "ok", nil
	})
	requireNoError(t, err)
	if result != "ok" {
		t.Fatalf("expected 'ok', got %q", result)
	}
	requireCallCount(t, calls, 2)
}

func TestRetryOnRateLimit_ServerError_ExhaustsRetries(t *testing.T) {
	calls := 0
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	_, err := RetryOnRateLimit(ctx, func() (string, error) {
		calls++
		return "", newServerError(http.StatusInternalServerError)
	})
	if err == nil {
		t.Fatal("expected error after exhausting retries")
	}
	requireCallCount(t, calls, maxRetries+1)
}

func TestRetryOnRateLimit_ClientError4xx_NotRetried(t *testing.T) {
	calls := 0
	_, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		return "", newServerError(http.StatusBadRequest)
	})
	if err == nil {
		t.Fatal("expected error")
	}
	requireCallCount(t, calls, 1)
}

func TestRetryOnRateLimit_ServerError_ExponentialBackoff(t *testing.T) {
	calls := 0
	start := time.Now()
	_, err := RetryOnRateLimit(context.Background(), func() (string, error) {
		calls++
		if calls <= 2 {
			return "", newServerError(http.StatusInternalServerError)
		}
		return "ok", nil
	})
	elapsed := time.Since(start)
	requireNoError(t, err)
	requireCallCount(t, calls, 3)
	// First wait: 2s, second wait: 4s = 6s total minimum
	if elapsed < 6*time.Second {
		t.Fatalf("expected at least 6s of exponential backoff, got %v", elapsed)
	}
}

func TestRetryOnRateLimit_ServerErrorNoResult_RetriesThenSucceeds(t *testing.T) {
	calls := 0
	err := RetryOnRateLimitNoResult(context.Background(), func() error {
		calls++
		if calls == 1 {
			return newServerError(http.StatusInternalServerError)
		}
		return nil
	})
	requireNoError(t, err)
	requireCallCount(t, calls, 2)
}

// Tests for isRetryableError

func TestIsRetryableError_RateLimit(t *testing.T) {
	de, ok := isRetryableError(newRateLimitError())
	if !ok || de == nil {
		t.Fatal("expected rate limit error to be retryable")
	}
}

func TestIsRetryableError_ServerError(t *testing.T) {
	de, ok := isRetryableError(newServerError(http.StatusInternalServerError))
	if !ok || de == nil {
		t.Fatal("expected 500 error to be retryable")
	}
}

func TestIsRetryableError_ClientError(t *testing.T) {
	_, ok := isRetryableError(newServerError(http.StatusBadRequest))
	if ok {
		t.Fatal("expected 400 error to not be retryable")
	}
}

func TestIsRetryableError_NonDescopeError(t *testing.T) {
	_, ok := isRetryableError(errors.New("generic error"))
	if ok {
		t.Fatal("expected non-descope error to not be retryable")
	}
}

func TestIsRetryableError_DescopeErrorNoStatusCode(t *testing.T) {
	_, ok := isRetryableError(&descope.Error{Code: "E999999", Message: "no status"})
	if ok {
		t.Fatal("expected descope error without status code to not be retryable")
	}
}

// Tests for retryWaitDuration

func TestRetryWaitDuration_RateLimitWithRetryAfter(t *testing.T) {
	de := newRateLimitError(5)
	wait := retryWaitDuration(de, 0)
	if wait != 5*time.Second {
		t.Fatalf("expected 5s, got %v", wait)
	}
}

func TestRetryWaitDuration_RateLimitDefault(t *testing.T) {
	de := newRateLimitError()
	wait := retryWaitDuration(de, 0)
	if wait != defaultRetryWait {
		t.Fatalf("expected %v, got %v", defaultRetryWait, wait)
	}
}

func TestRetryWaitDuration_ServerErrorExponential(t *testing.T) {
	de := newServerError(http.StatusInternalServerError)
	cases := []struct {
		attempt uint
		want    time.Duration
	}{
		{0, 2 * time.Second},
		{1, 4 * time.Second},
		{2, 8 * time.Second},
	}
	for _, tc := range cases {
		got := retryWaitDuration(de, tc.attempt)
		if got != tc.want {
			t.Fatalf("attempt %d: expected %v, got %v", tc.attempt, tc.want, got)
		}
	}
}

func TestRetryWaitDuration_CapsAtMax(t *testing.T) {
	de := newServerError(http.StatusInternalServerError)
	wait := retryWaitDuration(de, 10) // 2 << 10 = 2048s, should cap
	if wait != maxRetryWait {
		t.Fatalf("expected %v, got %v", maxRetryWait, wait)
	}
}
