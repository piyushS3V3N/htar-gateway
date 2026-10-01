/* Auto-generated eBPF XDP Driver Map Configuration by HTAR-Gateway Migrate */
#include <vmlinux.h>
#include <bpf/bpf_helpers.h>

struct {
    __uint(type, BPF_MAP_TYPE_HASH);
    __uint(max_entries, 10240);
    __type(key, __u32);   /* IPv4 Address */
    __type(value, __u8);  /* XDP Action Code */
} htar_xdp_blacklisted_ips SEC(".maps");

/* Route Rule 1: route_payments_service_v1_id */
