#include "tetra_sds.h"
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
    for(i=0;i<width;++i) {uint16_t bit=(uint16_t)(*offset+i);if((value>>(width-1u-i))&1u)bytes[bit/8u]|=(uint8_t)(1u<<(7u-bit%8u));}
    *offset=(uint16_t)(*offset+width);
}
static uint32_t get(const uint8_t *bytes,uint16_t offset,uint8_t width) {
    uint32_t value=0;uint8_t i;
    for(i=0;i<width;++i){uint16_t bit=(uint16_t)(offset+i);value=(value<<1)|((bytes[bit/8u]>>(7u-bit%8u))&1u);}
    return value;
}
static void downlink(struct tetra_sds_event *event,uint32_t source,const uint8_t *data,uint16_t bits) {
    uint16_t offset=0,i;
    clear(event,sizeof(*event));event->size=sizeof(*event);event->abi_version=TETRA_SDS_ABI_VERSION;event->kind=TETRA_SDS_DOWNLINK;
    put(event->pdu,&offset,15,5);put(event->pdu,&offset,1,2);put(event->pdu,&offset,source,24);
    put(event->pdu,&offset,3,2);put(event->pdu,&offset,bits,11);
    for(i=0;i<bits;++i)put(event->pdu,&offset,(data[i/8u]>>(7u-i%8u))&1u,1);
    put(event->pdu,&offset,0,1);event->pdu_bits=offset;
}
#define CHECK(x) do { if(!(x)) return __LINE__; } while(0)
#ifdef __cplusplus
extern "C"
#endif
int tetra_sds_c_smoke(void) {
    struct tetra_sds_storage storage;
    struct tetra_sds_config config={sizeof(config),TETRA_SDS_ABI_VERSION};
    struct tetra_sds_event event;
    struct tetra_sds_action action,before;
    struct tetra_sds_status status;
    const uint8_t transfer[]={0x82,0x04,9,0xa8},report[]={0x82,0x18,2,7};
    CHECK(tetra_sds_core_size()==sizeof(storage));
    CHECK(tetra_sds_init(&storage,sizeof(storage),&config)==TETRA_SDS_OK);
    clear(&event,sizeof(event));event.size=sizeof(event);event.abi_version=TETRA_SDS_ABI_VERSION;
    event.kind=TETRA_SDS_MESSAGE;event.ssi=555;event.ssi_type=2;event.protocol_id=0x82;
    event.reference=7;event.report_request=1;event.pdu_bits=5;event.pdu[0]=0xa8;
    CHECK(tetra_sds_push_event(&storage,&event)==TETRA_SDS_OK);
    event.pdu[0]=0;
    clear(&status,sizeof(status));status.size=sizeof(status);status.abi_version=TETRA_SDS_ABI_VERSION;
    CHECK(tetra_sds_get_status(&storage,&status)==TETRA_SDS_OK);
    CHECK(status.outstanding==1 && (status.present[0]&128u)!=0 && status.destination[7]==555);
    clear(&action,sizeof(action));action.size=sizeof(action);action.abi_version=99;
    CHECK(tetra_sds_poll_action(&storage,&action)==TETRA_SDS_ABI_MISMATCH);
    action.abi_version=TETRA_SDS_ABI_VERSION;CHECK(tetra_sds_poll_action(&storage,&action)==TETRA_SDS_OK);
    CHECK(action.kind==TETRA_SDS_UPLINK && action.ssi==555 && get(action.pdu,48,24)==0x820407);
    CHECK(get(action.pdu,72,5)==0x15);
    downlink(&event,777,transfer,29);
    CHECK(tetra_sds_push_event(&storage,&event)==TETRA_SDS_OK);
    CHECK(tetra_sds_poll_action(&storage,&action)==TETRA_SDS_OK);
    CHECK(action.kind==TETRA_SDS_MESSAGE_INDICATION && action.reference==9 && action.ssi==777 && action.pdu_bits==5 && action.pdu[0]==0xa8);
    CHECK(tetra_sds_poll_action(&storage,&action)==TETRA_SDS_OK);
    CHECK(action.kind==TETRA_SDS_UPLINK && action.ssi==777 && get(action.pdu,48,32)==0x82100009);
    downlink(&event,555,report,32);
    CHECK(tetra_sds_push_event(&storage,&event)==TETRA_SDS_OK);
    CHECK(tetra_sds_poll_action(&storage,&action)==TETRA_SDS_OK);
    CHECK(action.kind==TETRA_SDS_REPORT_INDICATION && action.reference==7 && action.delivery_status==2);
    CHECK(tetra_sds_poll_action(&storage,&action)==TETRA_SDS_OK);
    CHECK(action.kind==TETRA_SDS_UPLINK && get(action.pdu,48,32)==0x82200207);
    before=action;CHECK(tetra_sds_poll_action(&storage,&action)==TETRA_SDS_EMPTY);
    CHECK(same(&action,&before,sizeof(action)));
    CHECK(tetra_sds_get_status(&storage,&status)==TETRA_SDS_OK && status.outstanding==0);
    clear(&event,sizeof(event));event.size=sizeof(event);event.abi_version=TETRA_SDS_ABI_VERSION;
    event.kind=TETRA_SDS_CANCEL;event.reference=7;
    CHECK(tetra_sds_push_event(&storage,&event)==TETRA_SDS_EMPTY);
    event.kind=TETRA_SDS_UNITDATA;event.ssi=555;event.ssi_type=2;event.data_type=2;event.pdu_bits=64;
    event.pdu[0]=1;event.pdu[1]=0x23;event.pdu[2]=0x45;event.pdu[3]=0x67;
    event.pdu[4]=0x89;event.pdu[5]=0xab;event.pdu[6]=0xcd;event.pdu[7]=0xef;
    CHECK(tetra_sds_push_event(&storage,&event)==TETRA_SDS_OK);
    CHECK(tetra_sds_poll_action(&storage,&action)==TETRA_SDS_OK);
    CHECK(get(action.pdu,37,32)==0x01234567 && get(action.pdu,69,32)==0x89abcdef);
    CHECK(tetra_sds_reset(&storage,&config)==TETRA_SDS_OK);
    CHECK(tetra_sds_get_status(&storage,&status)==TETRA_SDS_OK && status.outstanding==0 && status.actions==0);
    CHECK(tetra_sds_poll_action(&storage,(struct tetra_sds_action*)&storage)==TETRA_SDS_ARGUMENT);
    return 0;
}
