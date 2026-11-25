#include "tetra_cmce.h"
static void clear(void *memory,size_t count) {
    uint8_t *bytes=(uint8_t*)memory;size_t i;for(i=0;i<count;++i)bytes[i]=0;
}
static int same(const void *left,const void *right,size_t count) {
    const uint8_t *a=(const uint8_t*)left,*b=(const uint8_t*)right;size_t i;
    for(i=0;i<count;++i) { if(a[i]!=b[i])return 0; }
    return 1;
}

static void put(uint8_t *bytes,uint16_t *offset,uint32_t value,uint8_t width) {
    uint8_t i;
    for(i=0;i<width;++i) {
        uint16_t bit=(uint16_t)(*offset+i);
        if((value>>(width-1u-i))&1u) bytes[bit/8u]|=(uint8_t)(1u<<(7u-bit%8u));
    }
    *offset=(uint16_t)(*offset+width);
}
#define CHECK(x) do { if(!(x)) return __LINE__; } while(0)
#ifdef __cplusplus
extern "C"
#endif
int tetra_cmce_c_smoke(void) {
    struct tetra_cmce_storage storage;
    struct tetra_cmce_config config={sizeof(config),TETRA_CMCE_ABI_VERSION,123,TETRA_CMCE_HAS_IDENTITY};
    struct tetra_cmce_event event;
    struct tetra_cmce_action action,before;
    struct tetra_cmce_status status;
    uint16_t offset;
    uint32_t notices=0,configures=0,calls=0;
    int32_t result;
    CHECK(tetra_cmce_core_size()==sizeof(storage));
    CHECK(tetra_cmce_core_align()==TETRA_CMCE_STORAGE_ALIGN);
    CHECK(tetra_cmce_init(&storage,sizeof(storage)-1u,&config)==TETRA_CMCE_CAPACITY);
    CHECK(tetra_cmce_init(&storage,sizeof(storage),&config)==TETRA_CMCE_OK);
    clear(&event,sizeof(event));
    event.size=sizeof(event);event.abi_version=TETRA_CMCE_ABI_VERSION;event.kind=TETRA_CMCE_ORIGINATE;
    event.route.ssi=91;event.route.ssi_type=3;event.basic.communication=1;event.flags=TETRA_CMCE_REQUEST_TX;
    CHECK(tetra_cmce_push_event(&storage,&event)==TETRA_CMCE_OK);
    clear(&action,sizeof(action));action.size=sizeof(action);action.abi_version=99;
    CHECK(tetra_cmce_poll_action(&storage,&action)==TETRA_CMCE_ABI_MISMATCH);
    action.abi_version=TETRA_CMCE_ABI_VERSION;
    CHECK(tetra_cmce_poll_action(&storage,&action)==TETRA_CMCE_OK);
    CHECK(action.kind==TETRA_CMCE_UPLINK && action.route.ssi==123 && action.route.ssi_type==2);
    CHECK((action.pdu[0]>>3)==7);
    clear(&event,sizeof(event));event.size=sizeof(event);event.abi_version=TETRA_CMCE_ABI_VERSION;
    event.kind=TETRA_CMCE_DOWNLINK;event.now=7;event.route.ssi=91;event.route.ssi_type=3;
    event.route.handle=11;event.route.endpoint_id=22;event.route.link_id=33;
    offset=0;
    put(event.pdu,&offset,7,5);put(event.pdu,&offset,7,14);put(event.pdu,&offset,1,4);
    put(event.pdu,&offset,0,1);put(event.pdu,&offset,0,1);put(event.pdu,&offset,4,8);
    put(event.pdu,&offset,0,2);put(event.pdu,&offset,0,1);put(event.pdu,&offset,0,4);
    put(event.pdu,&offset,1,1);put(event.pdu,&offset,0,1);put(event.pdu,&offset,0,1);
    put(event.pdu,&offset,1,1);put(event.pdu,&offset,1,2);put(event.pdu,&offset,456,24);put(event.pdu,&offset,0,1);
    event.pdu_bits=offset;
    CHECK(tetra_cmce_push_event(&storage,&event)==TETRA_CMCE_OK);
    clear(event.pdu,sizeof(event.pdu));
    action.size=sizeof(action);action.abi_version=TETRA_CMCE_ABI_VERSION;
    while((result=tetra_cmce_poll_action(&storage,&action))==TETRA_CMCE_OK) {
        if(action.kind==TETRA_CMCE_NOTIFY) {
            ++notices;CHECK(action.route.handle==11 && action.route.endpoint_id==22 && action.route.link_id==33);
            CHECK(action.fields.kind==7 && action.fields.speaker==456);
            CHECK(action.pdu_bits==offset && (action.pdu[0]>>3)==7);
        }
        if(action.kind==TETRA_CMCE_CONFIGURE) {++configures;CHECK((action.flags&TETRA_CMCE_SWITCH_U_PLANE)!=0);CHECK((action.flags&TETRA_CMCE_TX_GRANT)==0);}
        if(action.kind==TETRA_CMCE_CALL) {++calls;CHECK(action.change==TETRA_CMCE_CALL_CREATED);}
    }
    CHECK(result==TETRA_CMCE_EMPTY && notices==2 && configures==1 && calls==1);
    before=action;CHECK(tetra_cmce_poll_action(&storage,&action)==TETRA_CMCE_EMPTY);
    CHECK(same(&action,&before,sizeof(action)));
    clear(&event,sizeof(event));event.size=sizeof(event);event.abi_version=TETRA_CMCE_ABI_VERSION;
    event.kind=TETRA_CMCE_TRAFFIC;event.flags=TETRA_CMCE_OWNER_PRESENT;event.ssi=91;event.now=9;
    CHECK(tetra_cmce_push_event(&storage,&event)==TETRA_CMCE_OK);
    CHECK(tetra_cmce_poll_action(&storage,&action)==TETRA_CMCE_OK);
    CHECK(action.kind==TETRA_CMCE_SPEECH && action.id==7 && action.sequence==1 && action.speaker==456);
    event.kind=TETRA_CMCE_TRANSMITTED;event.flags=0;event.id=7;
    CHECK(tetra_cmce_push_event(&storage,&event)==TETRA_CMCE_NOT_ALLOWED);
    event.kind=TETRA_CMCE_DEMAND;event.priority=1;
    CHECK(tetra_cmce_push_event(&storage,&event)==TETRA_CMCE_OK);
    CHECK(tetra_cmce_poll_action(&storage,&action)==TETRA_CMCE_OK);
    CHECK(action.kind==TETRA_CMCE_UPLINK && action.route.ssi==123 && action.route.link_id==2);
    CHECK((action.flags&(TETRA_CMCE_STEALING|TETRA_CMCE_REPEATS))==3);
    clear(&status,sizeof(status));status.size=sizeof(status);status.abi_version=TETRA_CMCE_ABI_VERSION;
    CHECK(tetra_cmce_get_status(&storage,&status)==TETRA_CMCE_OK);
    CHECK(status.calls==1 && status.call[0].rx_frames==1 && status.call[0].tx_frames==0);
    CHECK(tetra_cmce_reset(&storage,&config)==TETRA_CMCE_OK);
    CHECK(tetra_cmce_get_status(&storage,&status)==TETRA_CMCE_OK);
    CHECK(status.calls==0 && status.actions==0 && status.originations==0);
    CHECK(tetra_cmce_poll_action(&storage,(struct tetra_cmce_action*)&storage)==TETRA_CMCE_ARGUMENT);
    return 0;
}
