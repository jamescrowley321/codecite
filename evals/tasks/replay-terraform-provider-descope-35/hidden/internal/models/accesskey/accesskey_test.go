package accesskey_test

import (
	"regexp"
	"testing"

	"github.com/descope/terraform-provider-descope/tools/testacc"
	"github.com/hashicorp/terraform-plugin-testing/helper/resource"
)

func TestAccessKey(t *testing.T) {
	a := testacc.AccessKey(t)
	testacc.Run(t,
		// Test creating with status = "inactive" fails
		resource.TestStep{
			Config: a.Config(`
				status = "inactive"
			`),
			ExpectError: regexp.MustCompile(`Cannot set status`),
		},
		// Test expire_time exceeding int32 max fails validation
		resource.TestStep{
			Config: a.Config(`
				expire_time = 9999999999
			`),
			ExpectError: regexp.MustCompile(`must be at most`),
		},
		// Test basic creation with company-level roles
		resource.TestStep{
			Config: a.Config(`
				role_names = ["Tenant Admin"]
			`),
			Check: a.Check(map[string]any{
				"id":           testacc.AttributeIsSet,
				"name":         a.Name,
				"status":       "active",
				"cleartext":    testacc.AttributeIsSet,
				"client_id":    testacc.AttributeIsSet,
				"role_names.#": "1",
			}),
		},
		// Test status update
		resource.TestStep{
			Config: a.Config(`
				status = "inactive"
				role_names = ["Tenant Admin"]
			`),
			Check: a.Check(map[string]any{
				"id":     testacc.AttributeIsSet,
				"name":   a.Name,
				"status": "inactive",
			}),
		},
		// Test import
		resource.TestStep{
			ResourceName: a.Path(),
			ImportState:  true,
		},
		// Test with description, permitted_ips, and custom_claims
		resource.TestStep{
			Config: a.Config(`
				description = "Test access key"
				permitted_ips = ["192.168.1.0/24"]
				role_names = ["Tenant Admin"]
				custom_claims = {
					"claim1" = "value1"
				}
			`),
			Check: a.Check(map[string]any{
				"description":          "Test access key",
				"permitted_ips.#":      "1",
				"permitted_ips.0":      "192.168.1.0/24",
				"custom_claims.%":      "1",
				"custom_claims.claim1": "value1",
			}),
		},
		// Destroy resource
		resource.TestStep{
			Config: a.Config(`
				role_names = ["Tenant Admin"]
			`),
			Destroy: true,
		},
	)
}
