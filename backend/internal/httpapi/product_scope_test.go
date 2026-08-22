package httpapi

import (
	"net/http/httptest"
	"testing"

	"autoLive/backend/internal/controlplane"
)

func TestResolveAdminProductScope(t *testing.T) {
	localAdmin := controlplane.Actor{UserID: "usr_local_admin", Role: "admin", Product: controlplane.ProductAutoLive}
	ordinaryAdmin := controlplane.Actor{UserID: "usr_other_admin", Role: "admin", Product: controlplane.ProductAutoLive}

	for _, test := range []struct {
		name    string
		actor   controlplane.Actor
		query   string
		want    controlplane.ProductCode
		wantErr error
	}{
		{name: "local admin may query all products", actor: localAdmin, want: ""},
		{name: "local admin may narrow product", actor: localAdmin, query: "product=douyin_desktop", want: controlplane.ProductDouyinDesktop},
		{name: "ordinary admin defaults to session product", actor: ordinaryAdmin, want: controlplane.ProductAutoLive},
		{name: "ordinary admin may request own product", actor: ordinaryAdmin, query: "product=autolive", want: controlplane.ProductAutoLive},
		{name: "ordinary admin may not widen product", actor: ordinaryAdmin, query: "product=douyin_desktop", wantErr: controlplane.ErrForbidden},
		{name: "invalid product is rejected", actor: localAdmin, query: "product=unknown", wantErr: controlplane.ErrInvalidRequest},
		{name: "repeated product is rejected", actor: localAdmin, query: "product=autolive&product=douyin_desktop", wantErr: controlplane.ErrInvalidRequest},
	} {
		t.Run(test.name, func(t *testing.T) {
			req := httptest.NewRequest("GET", "/api/v1/admin/devices?"+test.query, nil)
			got, err := resolveAdminProductScope(req, test.actor)
			if err != test.wantErr {
				t.Fatalf("resolveAdminProductScope() error = %v, want %v", err, test.wantErr)
			}
			if got != test.want {
				t.Fatalf("resolveAdminProductScope() = %q, want %q", got, test.want)
			}
		})
	}
}
