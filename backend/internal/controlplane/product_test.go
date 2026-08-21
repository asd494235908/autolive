package controlplane

import (
	"encoding/json"
	"testing"
)

func TestParseProductCodeAcceptsOnlyRegisteredProducts(t *testing.T) {
	for _, raw := range []string{"autolive", "douyin_desktop"} {
		product, err := ParseProductCode(raw)
		if err != nil || !product.Valid() {
			t.Fatalf("ParseProductCode(%q) = %q, %v", raw, product, err)
		}
	}
	for _, raw := range []string{"", "AutoLive", "douyin-desktop", "unknown"} {
		if _, err := ParseProductCode(raw); err == nil {
			t.Fatalf("ParseProductCode(%q) unexpectedly accepted", raw)
		}
	}
}

func TestProductFieldsRoundTripThroughControlPlaneDTOs(t *testing.T) {
	device := DeviceRegistration{
		Product:    ProductDouyinDesktop,
		DeviceID:   "dev_00000001",
		DeviceName: "desktop",
		Platform:   "windows",
		AppVersion: "2.0.0",
	}
	encoded, err := json.Marshal(device)
	if err != nil {
		t.Fatal(err)
	}
	var decoded DeviceRegistration
	if err := json.Unmarshal(encoded, &decoded); err != nil {
		t.Fatal(err)
	}
	if decoded.Product != ProductDouyinDesktop {
		t.Fatalf("product = %q", decoded.Product)
	}
}
