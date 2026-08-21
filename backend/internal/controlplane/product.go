package controlplane

import (
	"fmt"
	"strings"
)

// ProductCode identifies one registered product in the control plane.
type ProductCode string

const (
	ProductAutoLive      ProductCode = "autolive"
	ProductDouyinDesktop ProductCode = "douyin_desktop"
)

func (p ProductCode) Valid() bool {
	switch p {
	case ProductAutoLive, ProductDouyinDesktop:
		return true
	default:
		return false
	}
}

func ParseProductCode(raw string) (ProductCode, error) {
	product := ProductCode(strings.TrimSpace(raw))
	if !product.Valid() {
		return "", fmt.Errorf("invalid product code %q", raw)
	}
	return product, nil
}

type ProductSummary struct {
	Code      ProductCode `json:"code"`
	Status    string      `json:"status"`
	CreatedAt string      `json:"created_at"`
}

type UserProductMembership struct {
	UserID              string      `json:"user_id"`
	Product             ProductCode `json:"product"`
	Status              string      `json:"status"`
	EntitlementRevision int64       `json:"entitlement_revision"`
	CreatedAt           string      `json:"created_at"`
	UpdatedAt           string      `json:"updated_at"`
}
