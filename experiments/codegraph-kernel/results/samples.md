# Sampled disagreements and headers

Seed 20261125. Lines are from the pinned checkouts. Verdicts are in judgements.json.

## c: codecite_only (1503 in all)

### c/codecite_only/1: linux `arch/mips/include/asm/mach-au1x00/au1xxx_dbdma.h` 55-64 struct `au1xxx_dma_channel`

```
    55  typedef volatile struct au1xxx_dma_channel {
    56  	u32	ddma_cfg;	/* See below */
    57  	u32	ddma_desptr;	/* 32-byte aligned pointer to descriptor */
    58  	u32	ddma_statptr;	/* word aligned pointer to status word */
    59  	u32	ddma_dbell;	/* A write activates channel operation */
    60  	u32	ddma_irq;	/* If bit 0 set, interrupt pending */
```

### c/codecite_only/2: linux `drivers/gpu/drm/radeon/atombios.h` 4366-4370 struct `_ATOM_CONNECTOR_HARDCODE_DTD_RECORD`

```
  4366  typedef struct  _ATOM_CONNECTOR_HARDCODE_DTD_RECORD
  4367  {
  4368    ATOM_COMMON_RECORD_HEADER   sheader;
  4369  	ATOM_DTD_FORMAT							asTiming;
  4370  }ATOM_CONNECTOR_HARDCODE_DTD_RECORD;
```

### c/codecite_only/3: linux `drivers/gpu/drm/radeon/atombios.h` 3321-3336 struct `_SET_CRTC_USING_DTD_TIMING_PARAMETERS`

```
  3321  typedef struct _SET_CRTC_USING_DTD_TIMING_PARAMETERS
  3322  {
  3323    USHORT  usH_Size;
  3324    USHORT  usH_Blanking_Time;
  3325    USHORT  usV_Size;
  3326    USHORT  usV_Blanking_Time;			
```

### c/codecite_only/4: linux `drivers/scsi/mpt3sas/mpi/mpi2_ioc.h` 1353-1368 type_alias `Mpi2EventAckReply_t`

```
  1353  /*EventAck Reply message */
  1354  typedef struct _MPI2_EVENT_ACK_REPLY {
  1355  	U16 Reserved1;		/*0x00 */
  1356  	U8 MsgLength;		/*0x02 */
  1357  	U8 Function;		/*0x03 */
  1358  	U16 Reserved2;		/*0x04 */
```

### c/codecite_only/5: linux `drivers/gpu/drm/amd/include/atombios.h` 7312-7318 struct `_INTERRUPT_SERVICE_PARAMETERS_V2`

```
  7312  typedef struct _INTERRUPT_SERVICE_PARAMETERS_V2
  7313  {
  7314    UCHAR  ucInterruptId;
  7315    UCHAR  ucServiceId;
  7316    UCHAR  ucStatus;
  7317    UCHAR  ucReserved;
```

### c/codecite_only/6: linux `arch/mips/include/asm/sn/sn0/hubio.h` 174-183 struct `hubii_wid_u`

```
   174  typedef union hubii_wid_u {
   175  	u64	wid_reg_value;
   176  	struct {
   177  		u64	wid_rsvd:	32,	/* unused */
   178  			wid_rev_num:	 4,	/* revision number */
   179  			wid_part_num:	16,	/* the widget type: hub=c101 */
```

### c/codecite_only/7: linux `drivers/scsi/mpt3sas/mpi/mpi2.h` 437-443 struct `_MPI2_SCSI_TARGET_REQUEST_DESCRIPTOR`

```
   437  typedef struct _MPI2_SCSI_TARGET_REQUEST_DESCRIPTOR {
   438  	U8 RequestFlags;	/*0x00 */
   439  	U8 MSIxIndex;		/*0x01 */
   440  	U16 SMID;		/*0x02 */
   441  	U16 LMID;		/*0x04 */
   442  	U16 IoIndex;		/*0x06 */
```

### c/codecite_only/8: linux `drivers/gpu/drm/radeon/atombios.h` 6698-6726 struct `_ATOM_VRAM_MODULE_V7`

```
  6698  typedef struct _ATOM_VRAM_MODULE_V7
  6699  {
  6700  // Design Specific Values
  6701    ULONG	  ulChannelMapCfg;	                // mmMC_SHARED_CHREMAP
  6702    USHORT  usModuleSize;                     // Size of ATOM_VRAM_MODULE_V7
  6703    USHORT  usPrivateReserved;                // MC_ARB_RAMCFG (includes NOOFBANK,NOOFRANKS,NOOFROWS,NOOFCOLS)
```

### c/codecite_only/9: linux `drivers/gpu/drm/amd/include/atombios.h` 4992-5003 struct `_ATOM_VOLTAGE_INFO_HEADER`

```
  4992  typedef struct  _ATOM_VOLTAGE_INFO_HEADER
  4993  {
  4994     USHORT   usVDDCBaseLevel;                //In number of 50mv unit
  4995     USHORT   usReserved;                     //For possible extension table offset
  4996     UCHAR    ucNumOfVoltageEntries;
  4997     UCHAR    ucBytesPerVoltageEntry;
```

### c/codecite_only/10: linux `drivers/target/target_core_pscsi.h` 45-48 enum `phv_modes`

```
    45  typedef enum phv_modes {
    46  	PHV_VIRTUAL_HOST_ID,
    47  	PHV_LLD_SCSI_HOST_NO
    48  } phv_modes_t;
```

## c: kernel_only (123857 in all)

### c/kernel_only/1: linux `drivers/media/platform/verisilicon/rockchip_vpu2_regs.h` 470-471 constant `VDPU_REG_DEC_CTRL5_IDR_PIC_ID` (qn `VDPU_REG_DEC_CTRL5_IDR_PIC_ID`)

```
   470  #define     VDPU_REG_DEC_CTRL5_IDR_PIC_ID(x)		(((x) & 0xffff) << 0)
   471  #define VDPU_REG_DEC_CTRL6			0x1c8
```

### c/kernel_only/2: linux `drivers/usb/serial/kl5kusb105.c` 69-72 constant `id_table` (qn `id_table`)

```
    69  static const struct usb_device_id id_table[] = {
    70  	{ USB_DEVICE(PALMCONNECT_VID, PALMCONNECT_PID) },
    71  	{ }		/* Terminating entry */
    72  };
```

### c/kernel_only/3: linux `drivers/clk/qcom/camcc-sm8450.c` 849-862 variable `cam_cc_camnoc_axi_clk_src` (qn `cam_cc_camnoc_axi_clk_src`)

```
   849  static struct clk_rcg2 cam_cc_camnoc_axi_clk_src = {
   850  	.cmd_rcgr = 0x13194,
   851  	.mnd_width = 0,
   852  	.hid_width = 5,
   853  	.parent_map = cam_cc_parent_map_0,
   854  	.freq_tbl = ftbl_cam_cc_camnoc_axi_clk_src,
```

### c/kernel_only/4: linux `drivers/gpu/drm/i915/display/skl_universal_plane_regs.h` 217-220 constant `PLANE_AUX_OFFSET` (qn `PLANE_AUX_OFFSET`)

```
   217  #define PLANE_AUX_OFFSET(pipe, plane)	_MMIO_SKL_PLANE((pipe), (plane), \
   218  							_PLANE_AUX_OFFSET_1_A, _PLANE_AUX_OFFSET_1_B, \
   219  							_PLANE_AUX_OFFSET_2_A, _PLANE_AUX_OFFSET_2_B)
   220  
```

### c/kernel_only/5: linux `tools/testing/selftests/vfio/lib/include/vfio_util.h` 270-271 constant `vfio_pci_config_writel` (qn `vfio_pci_config_writel`)

```
   270  #define vfio_pci_config_writel(_d, _o, _v) vfio_pci_config_write(_d, _o, _v, u32)
   271  
```

### c/kernel_only/6: linux `drivers/net/wireless/intel/iwlwifi/iwl-config.h` 621-621 constant `iwl7260_n_name` (qn `iwl7260_n_name`)

```
   621  extern const char iwl7260_n_name[];
```

### c/kernel_only/7: linux `drivers/media/i2c/ds90ub953.h` 39-40 constant `UB953_REG_GPIO_INPUT_CTRL_OUT_EN` (qn `UB953_REG_GPIO_INPUT_CTRL_OUT_EN`)

```
    39  #define UB953_REG_GPIO_INPUT_CTRL_OUT_EN(n)	BIT(4 + (n))
    40  #define UB953_REG_GPIO_INPUT_CTRL_INPUT_EN(n)	BIT(0 + (n))
```

### c/kernel_only/8: linux `include/uapi/linux/if_pppox.h` 68-71 union `<anonymous>` (qn `sockaddr_pppox::<anonymous>`)

```
    68  	union {
    69  		struct pppoe_addr  pppoe;
    70  		struct pptp_addr   pptp;
    71  	} sa_addr;
```

### c/kernel_only/9: linux `drivers/staging/media/sunxi/sun6i-isp/sun6i_isp_reg.h` 154-155 constant `SUN6I_ISP_OB_VALID_HEIGHT` (qn `SUN6I_ISP_OB_VALID_HEIGHT`)

```
   154  #define SUN6I_ISP_OB_VALID_HEIGHT(v)		(((v) << 16) & GENMASK(28, 16))
   155  
```

### c/kernel_only/10: linux `arch/powerpc/include/asm/nohash/64/pgtable-4k.h` 90-92 constant `remap_4k_pfn` (qn `remap_4k_pfn`)

```
    90  #define remap_4k_pfn(vma, addr, pfn, prot)	\
    91  	remap_pfn_range((vma), (addr), (pfn), PAGE_SIZE, (prot))
    92  
```

## cpp: kernel_only (16 in all)

### cpp/kernel_only/1: linux `tools/testing/selftests/bpf/test_cpp.cpp` 42-42 method `load` (qn `Skeleton::load`)

```
    42  	int load() { return T::load(skel); }
```

### cpp/kernel_only/2: linux `tools/testing/selftests/bpf/test_cpp.cpp` 50-50 method `operator->` (qn `Skeleton::operator->`)

```
    50  	T* operator->() { return skel; }
```

### cpp/kernel_only/3: linux `tools/testing/selftests/bpf/test_cpp.cpp` 102-139 function `main` (qn `main`)

```
   102  int main(int argc, char *argv[])
   103  {
   104  	struct btf_dump_opts opts = { };
   105  	struct test_core_extern *skel;
   106  	struct struct_ops_module *skel2;
   107  	struct btf *btf;
```

### cpp/kernel_only/4: linux `tools/testing/selftests/bpf/test_cpp.cpp` 25-40 method `open` (qn `Skeleton::open`)

```
    25  	int open(const struct bpf_object_open_opts *opts = nullptr)
    26  	{
    27  		int err;
    28  
    29  		if (skel)
    30  			return -EBUSY;
```

### cpp/kernel_only/5: linux `tools/build/feature/test-cxa-demangle.cpp` 6-17 function `main` (qn `main`)

```
     6  int main(void)
     7  {
     8    size_t len = 256;
     9    char *output = (char*)malloc(len);
    10          int status;
    11  
```

### cpp/kernel_only/6: linux `tools/testing/selftests/bpf/test_cpp.cpp` 48-48 method `operator->` (qn `Skeleton::operator->`)

```
    48  	const T* operator->() const { return skel; }
```

### cpp/kernel_only/7: linux `tools/testing/selftests/bpf/test_cpp.cpp` 23-23 method `~Skeleton` (qn `Skeleton::~Skeleton`)

```
    23  	~Skeleton() { if (skel) T::destroy(skel); }
```

### cpp/kernel_only/8: linux `tools/testing/selftests/bpf/test_cpp.cpp` 21-21 method `Skeleton` (qn `Skeleton::Skeleton`)

```
    21  	Skeleton(): skel(nullptr) { }
```

### cpp/kernel_only/9: linux `tools/build/feature/test-llvm.cpp` 9-14 function `main` (qn `main`)

```
     9  int main()
    10  {
    11  	llvm::errs() << "Hello World!\n";
    12  	llvm::llvm_shutdown();
    13  	return 0;
    14  }
```

### cpp/kernel_only/10: linux `tools/testing/selftests/bpf/test_cpp.cpp` 52-52 method `get` (qn `Skeleton::get`)

```
    52  	const T *get() const { return skel; }
```

## java: codecite_only (7 in all)

### java/codecite_only/1: keycloak `quarkus/config-api/src/main/java/org/keycloak/config/database/Database.java` 106-129 method `apply`

```
   106                      @Override
   107                      public String apply(Function<Option<?>, String> getter, String namedProperty, String alias) {
   108                          String url;
   109                          if ("dev-file".equalsIgnoreCase(alias)) {
   110                              var separator = escapeReplacements(File.separator);
   111                              url = new StringBuilder()
```

### java/codecite_only/2: keycloak `quarkus/config-api/src/main/java/org/keycloak/config/database/Database.java` 139-147 method `escapeReplacements`

```
   139                      private String escapeReplacements(String snippet) {
   140                          if (File.separator.equals("\\")) {
   141                              // SmallRye will do replacements of "${...}", but a "\" must not escape such an expression.
   142                              // As we nest multiple expressions, and each nested expression must re-escape the backslashes,
   143                              // the simplest way is to replace a backslash with a slash, as those are processed nicely on Windows.
   144                              return snippet.replace("\\", "/");
```

### java/codecite_only/3: keycloak `quarkus/config-api/src/main/java/org/keycloak/config/database/Database.java` 135-137 method `getDbName`

```
   135                      private String getDbName(String namedProperty) {
   136                          return StringUtil.isNullOrEmpty(namedProperty) ? "keycloakdb" : "keycloakdb-%s".formatted(namedProperty);
   137                      }
```

### java/codecite_only/4: keycloak `quarkus/config-api/src/main/java/org/keycloak/config/database/Database.java` 131-133 method `getFolder`

```
   131                      private String getFolder(String namedProperty) {
   132                          return StringUtil.isNullOrEmpty(namedProperty) ? "h2" : "h2-%s".formatted(namedProperty);
   133                      }
```

### java/codecite_only/5: keycloak `services/src/test/java/org/keycloak/exportimport/ExportImportManagerTest.java` 31-34 method `create`

```
    31              @Override
    32              public KeycloakSession create() {
    33                  return null;
    34              }
```

### java/codecite_only/6: keycloak `testsuite/integration-arquillian/tests/base/src/test/java/org/keycloak/testsuite/authz/PolicyEvaluationTest.java` 723-726 method `getAttributes`

```
   723              @Override
   724              public Attributes getAttributes() {
   725                  return null;
   726              }
```

### java/codecite_only/7: keycloak `testsuite/integration-arquillian/tests/base/src/test/java/org/keycloak/testsuite/authz/PolicyEvaluationTest.java` 718-721 method `getId`

```
   718              @Override
   719              public String getId() {
   720                  return null;
   721              }
```

## java: kernel_only (11941 in all)

### java/kernel_only/1: keycloak `services/src/main/java/org/keycloak/authentication/forms/RegistrationPage.java` 48-48 constant `FIELD_PASSWORD` (qn `org.keycloak.authentication.forms::RegistrationPage::FIELD_PASSWORD`)

```
    48      public static final String FIELD_PASSWORD = "password";
```

### java/kernel_only/2: keycloak `services/src/main/java/org/keycloak/authentication/forms/RegistrationRecaptcha.java` 50-50 constant `SECRET_KEY` (qn `org.keycloak.authentication.forms::RegistrationRecaptcha::SECRET_KEY`)

```
    50      public static final String SECRET_KEY = "secret.key";
```

### java/kernel_only/3: keycloak `testsuite/integration-arquillian/tests/base/src/test/java/org/keycloak/testsuite/broker/OidcUserInfoClaimToRoleMapperTest.java` 32-32 constant `ATTRIBUTE_TO_MAP_USER_INFO` (qn `org.keycloak.testsuite.broker::OidcUserInfoClaimToRoleMapperTest::ATTRIBUTE_TO_MAP_USER_INFO`)

```
    32      protected static final String ATTRIBUTE_TO_MAP_USER_INFO = "user-attribute-info";
```

### java/kernel_only/4: keycloak `scim/services/src/main/java/org/keycloak/scim/services/Error.java` 27-27 constant `logger` (qn `org.keycloak.scim.services::Error::logger`)

```
    27      private static final Logger logger = Logger.getLogger(Error.class);
```

### java/kernel_only/5: keycloak `tests/base/src/test/java/org/keycloak/tests/admin/authz/fgap/RealmAdminAccessTest.java` 845-845 class `<GenericType$anon@845>` (qn `org.keycloak.tests.admin.authz.fgap::RealmAdminAccessTest::evaluateScopeMappings::<GenericType$anon@845>`)

```
   845              List<RoleRepresentation> roles = response.readEntity(new GenericType<List<RoleRepresentation>>() {});
```

### java/kernel_only/6: keycloak `server-spi-private/src/main/java/org/keycloak/transaction/RequestContextHelper.java` 39-39 constant `SESSION_ATTRIBUTE` (qn `org.keycloak.transaction::RequestContextHelper::SESSION_ATTRIBUTE`)

```
    39      public static final String SESSION_ATTRIBUTE = "REQ_CONTEXT_HELPER";
```

### java/kernel_only/7: keycloak `adapters/saml/wildfly/wildfly-subsystem/src/main/java/org/keycloak/subsystem/adapter/saml/extension/ServiceProviderDefinition.java` 54-57 constant `LOGOUT_PAGE` (qn `org.keycloak.subsystem.adapter.saml.extension::ServiceProviderDefinition::LOGOUT_PAGE`)

```
    54      private static final SimpleAttributeDefinition LOGOUT_PAGE =
    55              new SimpleAttributeDefinitionBuilder(Constants.Model.LOGOUT_PAGE, ModelType.STRING, true)
    56                      .setXmlName(Constants.XML.LOGOUT_PAGE)
    57                      .build();
```

### java/kernel_only/8: keycloak `server-spi-private/src/main/java/org/keycloak/scripting/ScriptBindingsConfigurer.java` 32-38 class `<ScriptBindingsConfigurer$anon@32>` (qn `org.keycloak.scripting::ScriptBindingsConfigurer::EMPTY::<ScriptBindingsConfigurer$anon@32>`)

```
    32      ScriptBindingsConfigurer EMPTY = new ScriptBindingsConfigurer() {
    33  
    34          @Override
    35          public void configureBindings(Bindings bindings) {
    36              //NOOP
    37          }
```

### java/kernel_only/9: keycloak `tests/base/src/test/java/org/keycloak/tests/suites/DatabaseTest.java` 26-26 constant `TAG` (qn `org.keycloak.tests.suites::DatabaseTest::TAG`)

```
    26      String TAG = "database";
```

### java/kernel_only/10: keycloak `tests/base/src/test/java/org/keycloak/tests/model/CompositeRoleExpansionQueryCountTest.java` 32-32 constant `BRANCHING` (qn `org.keycloak.tests.model::CompositeRoleExpansionQueryCountTest::BRANCHING`)

```
    32      private static final int BRANCHING = 5;
```

## javascript: codecite_only (25 in all)

### javascript/codecite_only/1: keycloak `themes/src/main/resources/theme/base/login/resources/js/kcNumberFormat.js` 9-20 method `onAdd`

```
     9    onAdd(element) {
    10      const formatValue = () => {
    11        const format = element.getAttribute(`data-${KC_NUMBER_FORMAT}`);
    12        element.value = formatNumber(element.value, format);
    13      };
    14  
```

### javascript/codecite_only/2: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.min.js` 17-17 function `g`

```
    17   */var B={astral:!1,namespacing:!0},w={},j={},k={},S=[],O="default",R="class",_={default:/\\(?:0(?:[0-3][0-7]{0,2}|[4-7][0-7]?)?|[1-9]\d*|x[\dA-Fa-f]{2}|u(?:[\dA-Fa-f]{4}|{[\dA-Fa-f]+})|c[A-Za-z]|[\s\
```

### javascript/codecite_only/3: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.min.js` 1-1 function `b`

```
     1  !function(u){if("object"==typeof exports&&"undefined"!=typeof module)module.exports=u();else if("function"==typeof define&&define.amd)define([],u);else{("undefined"!=typeof window?window:"undefined"!=
```

### javascript/codecite_only/4: keycloak `themes/src/main/resources/theme/base/login/resources/js/kcNumberUnFormat.js` 9-18 method `onAdd`

```
     9      onAdd(element) {
    10          for (let form of document.forms) {
    11              form.addEventListener('submit', (event) => {
    12                  const rawFormat = element.getAttribute(`data-${KC_NUMBER_UNFORMAT}`);
    13                  if (rawFormat) {
    14                      element.value = formatNumber(element.value, rawFormat);
```

### javascript/codecite_only/5: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.js` 5498-5507 function `$getOwnPropertyDescriptor`

```
  5498  var $getOwnPropertyDescriptor = function getOwnPropertyDescriptor(O, P) {
  5499    var it = toIndexedObject(O);
  5500    var key = toPropertyKey(P);
  5501    if (it === ObjectPrototype && hasOwn(AllSymbols, key) && !hasOwn(ObjectPrototypeSymbols, key)) return;
  5502    var descriptor = nativeGetOwnPropertyDescriptor(it, key);
  5503    if (descriptor && hasOwn(AllSymbols, key) && !(hasOwn(it, HIDDEN) && it[HIDDEN][key])) {
```

### javascript/codecite_only/6: excalidraw `scripts/woff2/woff2-vite-plugins.js` 16-18 method `config`

```
    16      config(_, { command }) {
    17        isDev = command === "serve";
    18      },
```

### javascript/codecite_only/7: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.js` 5509-5516 function `$getOwnPropertyNames`

```
  5509  var $getOwnPropertyNames = function getOwnPropertyNames(O) {
  5510    var names = nativeGetOwnPropertyNames(toIndexedObject(O));
  5511    var result = [];
  5512    $forEach(names, function (key) {
  5513      if (!hasOwn(AllSymbols, key) && !hasOwn(hiddenKeys, key)) push(result, key);
  5514    });
```

### javascript/codecite_only/8: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.js` 3992-4007 function `isConstructorLegacy`

```
  3992  var isConstructorLegacy = function isConstructor(argument) {
  3993    if (!isCallable(argument)) return false;
  3994    switch (classof(argument)) {
  3995      case 'AsyncFunction':
  3996      case 'GeneratorFunction':
  3997      case 'AsyncGeneratorFunction': return false;
```

### javascript/codecite_only/9: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.min.js` 12-12 function `b`

```
    12   */t.default=function _default(u){if(!u.addUnicodeData)throw new ReferenceError("Unicode Base must be loaded before Unicode Categories");u.addUnicodeData(i.default)},d.exports=t.default},{"../../tools
```

### javascript/codecite_only/10: excalidraw `dev-docs/docusaurus.config.js` 155-172 method `configureWebpack`

```
   155          configureWebpack() {
   156            return {
   157              module: {
   158                rules: [
   159                  {
   160                    test: /\.m?js$/,
```

## javascript: kernel_only (538 in all)

### javascript/kernel_only/1: django `django/contrib/admin/static/admin/js/vendor/select2/select2.full.js` 3985-3993 method `bind` (qn `MaximumSelectionLength::bind`)

```
  3985      function (decorated, container, $container) {
  3986        var self = this;
  3987  
  3988        decorated.call(this, container, $container);
  3989  
  3990        container.on('select', function () {
```

### javascript/kernel_only/2: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.min.js` 17-17 function `getNativeFlags` (qn `copyRegex::getNativeFlags`)

```
    17   */var B={astral:!1,namespacing:!0},w={},j={},k={},S=[],O="default",R="class",_={default:/\\(?:0(?:[0-3][0-7]{0,2}|[4-7][0-7]?)?|[1-9]\d*|x[\dA-Fa-f]{2}|u(?:[\dA-Fa-f]{4}|{[\dA-Fa-f]+})|c[A-Za-z]|[\s\
```

### javascript/kernel_only/3: django `django/contrib/admin/static/admin/js/nav_sidebar.js` 6-6 constant `main` (qn `main`)

```
     6          const main = document.getElementById('main');
```

### javascript/kernel_only/4: django `django/contrib/admin/static/admin/js/vendor/select2/select2.full.js` 5200-5209 method `set` (qn `Defaults::set`)

```
  5200    Defaults.prototype.set = function (key, value) {
  5201      var camelKey = $.camelCase(key);
  5202  
  5203      var data = {};
  5204      data[camelKey] = value;
  5205  
```

### javascript/kernel_only/5: django `js_tests/admin/jsi18n-mocks.test.js` 4-4 constant `django` (qn `django`)

```
     4      const django = globals.django;
```

### javascript/kernel_only/6: django `django/contrib/admin/static/admin/js/urlify.js` 92-96 constant `SERBIAN_MAP` (qn `SERBIAN_MAP`)

```
    92      const SERBIAN_MAP = {
    93          'ђ': 'dj', 'ј': 'j', 'љ': 'lj', 'њ': 'nj', 'ћ': 'c', 'џ': 'dz',
    94          'đ': 'dj', 'Ђ': 'Dj', 'Ј': 'j', 'Љ': 'Lj', 'Њ': 'Nj', 'Ћ': 'C',
    95          'Џ': 'Dz', 'Đ': 'Dj'
    96      };
```

### javascript/kernel_only/7: django `django/contrib/admin/static/admin/js/vendor/jquery/jquery.js` 3289-3302 function `remove` (qn `jQuery::Callbacks::self::remove`)

```
  3289  			remove: function() {
  3290  				jQuery.each( arguments, function( _, arg ) {
  3291  					var index;
  3292  					while ( ( index = jQuery.inArray( arg, list, index ) ) > -1 ) {
  3293  						list.splice( index, 1 );
  3294  
```

### javascript/kernel_only/8: django `django/contrib/admin/static/admin/js/vendor/jquery/jquery.js` 3443-3472 function `pipe` (qn `promise::pipe`)

```
  3443  				pipe: function( /* fnDone, fnFail, fnProgress */ ) {
  3444  					var fns = arguments;
  3445  
  3446  					return jQuery.Deferred( function( newDefer ) {
  3447  						jQuery.each( tuples, function( _i, tuple ) {
  3448  
```

### javascript/kernel_only/9: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.js` 3380-3380 function `values` (qn `values`)

```
  3380        defaultIterator = function values() { return call(nativeIterator, this); };
```

### javascript/kernel_only/10: django `django/contrib/admin/static/admin/js/vendor/select2/select2.full.js` 4814-5017 method `apply` (qn `Defaults::apply`)

```
  4814    Defaults.prototype.apply = function (options) {
  4815      options = $.extend(true, {}, this.defaults, options);
  4816  
  4817      if (options.dataAdapter == null) {
  4818        if (options.ajax != null) {
  4819          options.dataAdapter = AjaxData;
```

## python: kernel_only (720 in all)

### python/kernel_only/1: linux `tools/testing/selftests/net/bpf_offload.py` 1112-1112 variable `time_diff` (qn `time_diff`)

```
  1112      time_diff = end - start
```

### python/kernel_only/2: linux `tools/crypto/ccp/dbc_cli.py` 67-67 variable `sig` (qn `sig`)

```
    67      sig = None
```

### python/kernel_only/3: linux `tools/testing/selftests/net/bpf_offload.py` 754-754 variable `simdev` (qn `simdev`)

```
   754      simdev = BpfNetdevSimDev()
```

### python/kernel_only/4: linux `tools/workqueue/wq_dump.py` 248-248 variable `nna` (qn `nna`)

```
   248              nna = wq.node_nr_active[node]
```

### python/kernel_only/5: linux `tools/testing/selftests/x86/bugs/its_indirect_alignment.py` 126-126 variable `unknown` (qn `unknown`)

```
   126                          unknown = True
```

### python/kernel_only/6: linux `tools/power/pm-graph/sleepgraph.py` 7158-7158 variable `mode` (qn `mode`)

```
  7158  			mode = 'freeze'
```

### python/kernel_only/7: django `tests/timezones/tests.py` 57-57 variable `HAS_YAML` (qn `HAS_YAML`)

```
    57      HAS_YAML = False
```

### python/kernel_only/8: linux `tools/power/pm-graph/sleepgraph.py` 7033-7033 variable `val` (qn `val`)

```
  7033  				val = next(args)
```

### python/kernel_only/9: linux `tools/testing/selftests/net/rds/test.py` 235-235 variable `nr_success` (qn `nr_success`)

```
   235              nr_success = nr_success + 1
```

### python/kernel_only/10: linux `scripts/test_doc_build.py` 29-29 variable `min_python_bin` (qn `min_python_bin`)

```
    29          min_python_bin = p
```

## ruby: kernel_only (4 in all)

### ruby/kernel_only/1: ripgrep `pkg/brew/ripgrep-bin.rb` 1-23 class `RipgrepBin` (qn `RipgrepBin`)

```
     1  class RipgrepBin < Formula
     2    version '15.0.0'
     3    desc "Recursively search directories for a regex pattern."
     4    homepage "https://github.com/BurntSushi/ripgrep"
     5  
     6    if OS.mac?
```

### ruby/kernel_only/2: linux `tools/perf/Documentation/asciidoctor-extensions.rb` 6-23 class `LinkPerfProcessor` (qn `Perf::Documentation::LinkPerfProcessor`)

```
     6      class LinkPerfProcessor < Asciidoctor::Extensions::InlineMacroProcessor
     7        use_dsl
     8  
     9        named :chrome
    10  
    11        def process(parent, target, attrs)
```

### ruby/kernel_only/3: linux `tools/perf/Documentation/asciidoctor-extensions.rb` 11-22 method `process` (qn `Perf::Documentation::LinkPerfProcessor::process`)

```
    11        def process(parent, target, attrs)
    12          if parent.document.basebackend? 'html'
    13            %(<a href="#{target}.html">#{target}(#{attrs[1]})</a>\n)
    14          elsif parent.document.basebackend? 'manpage'
    15            "#{target}(#{attrs[1]})"
    16          elsif parent.document.basebackend? 'docbook'
```

### ruby/kernel_only/4: ripgrep `pkg/brew/ripgrep-bin.rb` 16-22 method `install` (qn `RipgrepBin::install`)

```
    16    def install
    17      bin.install "rg"
    18      man1.install "doc/rg.1"
    19  
    20      bash_completion.install "complete/rg.bash"
    21      zsh_completion.install "complete/_rg"
```

## rust: codecite_only (80 in all)

### rust/codecite_only/1: ripgrep `crates/core/flags/defs.rs` 3145-3146 variable `BORROWED`

```
  3145          static BORROWED: LazyLock<Vec<&'static str>> =
  3146              LazyLock::new(|| CHOICES.iter().map(|name| &**name).collect());
```

### rust/codecite_only/2: linux `drivers/gpu/nova-core/firmware.rs` 172-172 variable `BIN_MAGIC`

```
   172          const BIN_MAGIC: u32 = 0x10de;
```

### rust/codecite_only/3: ripgrep `crates/core/flags/parse.rs` 427-427 variable `THRESHOLD`

```
   427      const THRESHOLD: f64 = 0.4;
```

### rust/codecite_only/4: linux `drivers/android/binder/thread.rs` 697-698 variable `FD_FIELD_OFFSET`

```
   697                  const FD_FIELD_OFFSET: usize =
   698                      core::mem::offset_of!(uapi::binder_fd_object, __bindgen_anon_1.fd);
```

### rust/codecite_only/5: linux `rust/kernel/net/phy.rs` 574-576 variable `PHY_DEVICE_ID`

```
   574      /// This driver only works for PHYs with IDs which match this field.
   575      /// The default id and mask are zero.
   576      const PHY_DEVICE_ID: DeviceId = DeviceId::new_with_custom_mask(0, 0);
```

### rust/codecite_only/6: linux `rust/kernel/alloc/allocator.rs` 263-263 variable `TEST_LARGE_ALIGN_SIZE`

```
   263          const TEST_LARGE_ALIGN_SIZE: usize = kernel::page::PAGE_SIZE * 4;
```

### rust/codecite_only/7: linux `drivers/gpu/nova-core/falcon.rs` 291-292 variable `ID`

```
   291      /// Singleton of the engine, used to identify it with register I/O methods.
   292      const ID: Self;
```

### rust/codecite_only/8: linux `rust/kernel/cpufreq.rs` 722-723 variable `NAME`

```
   722      /// Driver's name.
   723      const NAME: &'static CStr;
```

### rust/codecite_only/9: linux `rust/kernel/debugfs/file_ops.rs` 118-118 variable `FILE_OPS`

```
   118      const FILE_OPS: FileOps<T>;
```

### rust/codecite_only/10: ripgrep `crates/cli/src/decompress.rs` 603-603 variable `ARGS_ZSTD`

```
   603      const ARGS_ZSTD: &[&str] = &["zstd", "-q", "-d", "-c", "--"];
```

## rust: kernel_only (91 in all)

### rust/kernel_only/1: linux `drivers/gpu/drm/tyr/regs.rs` 73-73 variable `Register` (qn `Register`)

```
    73  pub(crate) const GPU_TILER_PRESENT_HI: Register<0x114> = Register;
```

### rust/kernel_only/2: linux `rust/kernel/types.rs` 445-445 variable `PhantomData` (qn `PhantomData`)

```
   445  pub const NotThreadSafe: NotThreadSafe = PhantomData;
```

### rust/kernel_only/3: linux `drivers/gpu/drm/tyr/regs.rs` 74-74 variable `Register` (qn `Register`)

```
    74  pub(crate) const GPU_L2_PRESENT_LO: Register<0x120> = Register;
```

### rust/kernel_only/4: linux `drivers/gpu/drm/tyr/regs.rs` 99-99 variable `Register` (qn `Register`)

```
    99  pub(crate) const JOB_IRQ_CLEAR: Register<0x1004> = Register;
```

### rust/kernel_only/5: linux `drivers/gpu/nova-core/driver.rs` 22-22 variable `SZ_16M` (qn `SZ_16M`)

```
    22  const BAR0_SIZE: usize = SZ_16M;
```

### rust/kernel_only/6: linux `drivers/gpu/nova-core/fb/hal/tu102.rs` 57-57 variable `Tu102` (qn `Tu102`)

```
    57  const TU102: Tu102 = Tu102;
```

### rust/kernel_only/7: linux `rust/kernel/alloc.rs` 63-63 type_alias `Output` (qn `Flags::Output`)

```
    63      type Output = Self;
```

### rust/kernel_only/8: linux `samples/rust/rust_debugfs.rs` 103-103 variable `None` (qn `RustDebugFs::None`)

```
   103      const OF_ID_TABLE: Option<of::IdTable<Self::IdInfo>> = None;
```

### rust/kernel_only/9: linux `rust/kernel/rbtree.rs` 967-967 type_alias `IntoIter` (qn `RBTree::IntoIter`)

```
   967      type IntoIter = IterMut<'a, K, V>;
```

### rust/kernel_only/10: linux `drivers/gpu/drm/tyr/driver.rs` 187-187 variable `INFO` (qn `TyrDriver::INFO`)

```
   187      const INFO: drm::DriverInfo = INFO;
```

## tsx: codecite_only (30 in all)

### tsx/codecite_only/1: excalidraw `packages/excalidraw/actions/actionLinearEditor.tsx` 39-56 method `perform`

```
    39    perform(elements, appState, _, app) {
    40      const selectedElement = app.scene.getSelectedElements({
    41        selectedElementIds: appState.selectedElementIds,
    42        includeBoundTextElement: true,
    43      })[0] as ExcalidrawLinearElement;
    44  
```

### tsx/codecite_only/2: keycloak `js/apps/admin-ui/src/components/permission-tab/PermissionTab.tsx` 224-233 method `onClick`

```
   224                                onClick() {
   225                                  void navigate(
   226                                    toPermissionDetails({
   227                                      realm,
   228                                      id: realmId,
   229                                      permissionType: "scope",
```

### tsx/codecite_only/3: keycloak `js/apps/admin-ui/src/clients/ClientsSection.tsx` 238-241 method `onClick`

```
   238                      onClick() {
   239                        setSelectedClient(client);
   240                        toggleDeleteDialog();
   241                      },
```

### tsx/codecite_only/4: excalidraw `packages/excalidraw/components/ToolButton.tsx` 44-44 method `onClick`

```
    44        onClick?(event: React.MouseEvent): void;
```

### tsx/codecite_only/5: excalidraw `packages/excalidraw/actions/actionToggleObjectsSnapMode.tsx` 15-24 method `perform`

```
    15    perform(elements, appState) {
    16      return {
    17        appState: {
    18          ...appState,
    19          objectsSnapModeEnabled: !this.checked!(appState),
    20          gridModeEnabled: false,
```

### tsx/codecite_only/6: excalidraw `packages/excalidraw/components/ToolButton.tsx` 49-49 method `onClick`

```
    49        onClick?(): void;
```

### tsx/codecite_only/7: excalidraw `packages/excalidraw/frame.test.tsx` 11-11 type_alias `ElementType`

```
    11    type ElementType = string;
```

### tsx/codecite_only/8: excalidraw `packages/excalidraw/actions/actionToggleViewMode.tsx` 16-24 method `perform`

```
    16    perform(elements, appState) {
    17      return {
    18        appState: {
    19          ...appState,
    20          viewModeEnabled: !this.checked!(appState),
    21        },
```

### tsx/codecite_only/9: excalidraw `packages/excalidraw/components/App.tsx` 4055-4068 method `get`

```
  4055            get(ev: any, prop) {
  4056              const value = ev[prop];
  4057              if (typeof value === "function") {
  4058                // fix for Proxies hijacking `this`
  4059                return value.bind(ev);
  4060              }
```

### tsx/codecite_only/10: excalidraw `packages/excalidraw/components/ToolButton.tsx` 54-54 method `onChange`

```
    54        onChange?(data: { pointerType: PointerType | null }): void;
```

## tsx: kernel_only (591 in all)

### tsx/kernel_only/1: keycloak `js/apps/admin-ui/src/authentication/policies/WebauthnPolicy.tsx` 122-122 method `realmUpdated` (qn `WebauthnPolicyProps::realmUpdated`)

```
   122    realmUpdated: (realm: RealmRepresentation) => void;
```

### tsx/kernel_only/2: keycloak `js/apps/admin-ui/src/clients/keys/ExportSamlKeyDialog.tsx` 14-14 method `close` (qn `ExportSamlKeyDialogProps::close`)

```
    14    close: () => void;
```

### tsx/kernel_only/3: excalidraw `packages/excalidraw/components/App.tsx` 10481-10588 method `maybeDragNewGenericElement` (qn `App::maybeDragNewGenericElement`)

```
 10481    private maybeDragNewGenericElement = (
 10482      pointerDownState: PointerDownState,
 10483      event: MouseEvent | KeyboardEvent,
 10484      informMutation = true,
 10485    ): void => {
 10486      const selectionElement = this.state.selectionElement;
```

### tsx/kernel_only/4: excalidraw `excalidraw-app/App.tsx` 176-176 constant `parentUrl` (qn `parentUrl`)

```
   176      const parentUrl = new URL(document.referrer);
```

### tsx/kernel_only/5: keycloak `js/apps/admin-ui/src/groups/components/DeleteGroup.tsx` 12-12 method `refresh` (qn `DeleteConfirmProps::refresh`)

```
    12    refresh: () => void;
```

### tsx/kernel_only/6: keycloak `js/apps/account-ui/src/verifiable-credentials/CredentialRow.tsx` 35-35 method `t` (qn `RevokeDialogProps::t`)

```
    35    t: (key: string, params?: any) => string;
```

### tsx/kernel_only/7: keycloak `js/apps/admin-ui/src/authentication/DuplicateFlowModal.tsx` 22-22 method `toggleDialog` (qn `DuplicateFlowModalProps::toggleDialog`)

```
    22    toggleDialog: () => void;
```

### tsx/kernel_only/8: excalidraw `packages/excalidraw/components/LibraryMenu.tsx` 268-274 function `deselectItems` (qn `LibraryMenu::deselectItems`)

```
   268    const deselectItems = useCallback(() => {
   269      setAppState({
   270        selectedElementIds: {},
   271        selectedGroupIds: {},
   272        activeEmbeddable: null,
   273      });
```

### tsx/kernel_only/9: excalidraw `excalidraw-app/collab/Portal.tsx` 225-247 method `broadcastVisibleSceneBounds` (qn `Portal::broadcastVisibleSceneBounds`)

```
   225    broadcastVisibleSceneBounds = (
   226      payload: {
   227        sceneBounds: SocketUpdateDataSource["USER_VISIBLE_SCENE_BOUNDS"]["payload"]["sceneBounds"];
   228      },
   229      roomId: string,
   230    ) => {
```

### tsx/kernel_only/10: keycloak `js/apps/admin-ui/src/clients/authorization/NewPolicyDialog.tsx` 19-19 method `onSelect` (qn `NewPolicyDialogProps::onSelect`)

```
    19    onSelect: (provider: PolicyProviderRepresentation) => void;
```

## typescript: codecite_only (65 in all)

### typescript/codecite_only/1: excalidraw `packages/excalidraw/actions/actionToggleSearchMenu.ts` 19-49 method `perform`

```
    19    perform(elements, appState, _, app) {
    20      if (
    21        appState.openSidebar?.name === DEFAULT_SIDEBAR.name &&
    22        appState.openSidebar.tab === CANVAS_SEARCH_TAB
    23      ) {
    24        const searchInput =
```

### typescript/codecite_only/2: excalidraw `excalidraw-app/vite.config.mts` 56-63 method `assetFileNames`

```
    56            assetFileNames(chunkInfo) {
    57              if (chunkInfo?.name?.endsWith(".woff2")) {
    58                const family = chunkInfo.name.split("-")[0];
    59                return `fonts/${family}/[name][extname]`;
    60              }
    61  
```

### typescript/codecite_only/3: excalidraw `excalidraw-app/data/LocalData.ts` 152-184 method `getFiles`

```
   152      getFiles(ids) {
   153        return getMany(ids, filesStore).then(
   154          async (filesData: (BinaryFileData | undefined)[]) => {
   155            const loadedFiles: BinaryFileData[] = [];
   156            const erroredFiles = new Map<FileId, true>();
   157  
```

### typescript/codecite_only/4: excalidraw `examples/with-script-in-browser/utils.ts` 37-37 type_alias `RetType`

```
    37    type RetType = M extends false | undefined ? File : File[];
```

### typescript/codecite_only/5: excalidraw `packages/excalidraw/subset/woff2/woff2-bindings.ts` 3891-3895 method `get`

```
  3891          get() {
  3892            abort(
  3893              "'ALLOC_DYNAMIC' was not exported. add it to EXTRA_EXPORTED_RUNTIME_METHODS (see the FAQ)",
  3894            );
  3895          },
```

### typescript/codecite_only/6: excalidraw `packages/excalidraw/subset/woff2/woff2-bindings.ts` 304-306 method `get`

```
   304          get() {
   305            abort("Module.readBinary has been replaced with plain readBinary");
   306          },
```

### typescript/codecite_only/7: excalidraw `packages/excalidraw/subset/woff2/woff2-bindings.ts` 346-348 method `get`

```
   346          get() {
   347            abort("Module.wasmBinary has been replaced with plain wasmBinary");
   348          },
```

### typescript/codecite_only/8: excalidraw `packages/excalidraw/subset/woff2/woff2-bindings.ts` 3881-3885 method `get`

```
  3881          get() {
  3882            abort(
  3883              "'ALLOC_STACK' was not exported. add it to EXTRA_EXPORTED_RUNTIME_METHODS (see the FAQ)",
  3884            );
  3885          },
```

### typescript/codecite_only/9: keycloak `js/apps/admin-ui/vite.config.ts` 19-24 method `resolveId`

```
    19        resolveId(source) {
    20          if (source === "message-bundle") {
    21            return source;
    22          }
    23          return null;
    24        },
```

### typescript/codecite_only/10: excalidraw `excalidraw-app/data/LocalData.ts` 185-207 method `saveFiles`

```
   185      async saveFiles({ addedFiles }) {
   186        const savedFiles = new Map<FileId, BinaryFileData>();
   187        const erroredFiles = new Map<FileId, BinaryFileData>();
   188  
   189        // before we use `storage` event synchronization, let's update the flag
   190        // optimistically. Hopefully nothing fails, and an IDB read executed
```

## typescript: kernel_only (215 in all)

### typescript/kernel_only/1: keycloak `js/apps/admin-ui/src/components/dynamic/components.ts` 28-28 method `convertToName` (qn `ComponentProps::convertToName`)

```
    28    convertToName: (name: string) => string;
```

### typescript/kernel_only/2: keycloak `js/libs/keycloak-admin-client/src/resources/resource.ts` 30-41 method `makeUpdateRequest` (qn `Resource::makeUpdateRequest`)

```
    30    public makeUpdateRequest = <
    31      QueryType = any,
    32      PayloadType = any,
    33      ResponseType = any,
    34    >(
    35      args: RequestArgs,
```

### typescript/kernel_only/3: excalidraw `packages/excalidraw/tests/helpers/api.ts` 52-56 method `updateScene` (qn `API::updateScene`)

```
    52    static updateScene: InstanceType<typeof App>["updateScene"] = (...args) => {
    53      act(() => {
    54        h.app.updateScene(...args);
    55      });
    56    };
```

### typescript/kernel_only/4: keycloak `js/apps/account-ui/src/utils/useAccountAlerts.ts` 12-20 function `addAccountError` (qn `useAccountAlerts::addAccountError`)

```
    12      (messageKey: string, error: unknown) => {
    13        if (!(error instanceof ApiError)) {
    14          addError(messageKey, error);
    15          return;
    16        }
    17  
```

### typescript/kernel_only/5: excalidraw `excalidraw-app/data/FileManager.ts` 173-181 method `shouldPreventUnload` (qn `FileManager::shouldPreventUnload`)

```
   173    shouldPreventUnload = (elements: readonly ExcalidrawElement[]) => {
   174      return elements.some((element) => {
   175        return (
   176          isInitializedImageElement(element) &&
   177          !element.isDeleted &&
   178          this.savingFiles.has(element.fileId)
```

### typescript/kernel_only/6: excalidraw `packages/excalidraw/tests/helpers/api.ts` 66-70 method `setElements` (qn `API::setElements`)

```
    66    static setElements = (elements: readonly ExcalidrawElement[]) => {
    67      act(() => {
    68        h.elements = elements;
    69      });
    70    };
```

### typescript/kernel_only/7: excalidraw `packages/excalidraw/element/textWrapping.ts` 205-205 function `and` (qn `Regex::and`)

```
   205    and: (...regexes: RegExp[]): RegExp => Regex.build(Regex.join(...regexes)),
```

### typescript/kernel_only/8: excalidraw `packages/excalidraw/data/restore.ts` 658-668 function `isSidebarDocked` (qn `LegacyAppStateMigrations::isSidebarDocked`)

```
   658    isSidebarDocked: (appState, defaultAppState) => {
   659      return [
   660        "defaultSidebarDockedPreference",
   661        appState.isSidebarDocked ??
   662          coalesceAppStateValue(
   663            "defaultSidebarDockedPreference",
```

### typescript/kernel_only/9: excalidraw `packages/excalidraw/element/textWrapping.ts` 238-245 function `Before` (qn `Break::Before`)

```
   238    Before: (...regexes: RegExp[]) => {
   239      const joined = Regex.join(...regexes);
   240      const builder = () => Regex.build(`(?=[${joined}])`);
   241      return Break.Chain(builder) as Omit<
   242        ReturnType<typeof Break.Chain>,
   243        "FollowedBy"
```

### typescript/kernel_only/10: excalidraw `packages/excalidraw/tests/helpers/api.ts` 506-510 method `executeAction` (qn `API::executeAction`)

```
   506    static executeAction = (action: Action) => {
   507      act(() => {
   508        h.app.actionManager.executeAction(action);
   509      });
   510    };
```

## c: headers (193923 in all)

### c/header/1: linux `drivers/net/ethernet/engleder/tsnep_selftests.c` 33

- header: `function gc_timeout_signaled`
- doc: ""

```
    30  	return true;
    31  }
    32  
    33  static bool gc_timeout_signaled(struct tsnep_adapter *adapter)
    34  {
    35  	if (ioread32(adapter->addr + TSNEP_GC) & TSNEP_GC_TIMEOUT_SIGNAL)
```

### c/header/2: linux `include/linux/greybus/greybus_protocols.h` 1692

- header: `struct gb_lights_get_channel_config_response`
- doc: "channel config response payload"

```
  1689  } __packed;
  1690  
  1691  /* channel config response payload */
  1692  struct gb_lights_get_channel_config_response {
  1693  	__u8	max_brightness;
  1694  	__le32	flags;
```

### c/header/3: linux `drivers/net/ethernet/intel/ice/ice_ptp.h` 333

- header: `function ice_ptp_restore_timestamp_mode`
- doc: ""

```
   330  	return -EOPNOTSUPP;
   331  }
   332  
   333  static inline void ice_ptp_restore_timestamp_mode(struct ice_pf *pf) { }
   334  static inline void ice_ptp_extts_event(struct ice_pf *pf) { }
   335  static inline s8
```

### c/header/4: linux `drivers/misc/eeprom/eeprom_93cx6.c` 289

- header: `function eeprom_93cx6_multireadb`
- doc: "eeprom_93cx6_multireadb - Read multiple bytes from eeprom\n@eeprom: Pointer to eeprom structure\n@byte: Index from where we should start reading\n@data: target pointer where the information will have to be stored\n@bytes: Number of bytes that should be read.\nThis function will read all requested bytes f"

```
   286   * This function will read all requested bytes from the eeprom,
   287   * this is done by calling eeprom_93cx6_readb() multiple times.
   288   */
   289  void eeprom_93cx6_multireadb(struct eeprom_93cx6 *eeprom, const u8 byte,
   290  	u8 *data, const u16 bytes)
   291  {
```

### c/header/5: linux `drivers/devfreq/governor.h` 92

- header: `struct devfreq_governor`
- doc: "struct devfreq_governor - Devfreq policy governor\n@node:\t\tlist node - contains registered devfreq governors\n@name:\t\tGovernor's name\n@attrs:\t\tGovernor's sysfs attribute flags\n@flags:\t\tGovernor's feature flags\n@get_target_freq:\tReturns desired operating frequency for the device.\n\t\tBasically, get_targe"

```
    89   *
    90   * Note that the callbacks are called with devfreq->lock locked by devfreq.
    91   */
    92  struct devfreq_governor {
    93  	struct list_head node;
    94  
```

### c/header/6: linux `arch/powerpc/include/asm/spu_priv1.h` 86

- header: `function spu_mfc_dar_get -> u64`
- doc: ""

```
    83  	spu_priv1_ops->cpu_affinity_set(spu, cpu);
    84  }
    85  
    86  static inline u64
    87  spu_mfc_dar_get (struct spu *spu)
    88  {
```

### c/header/7: linux `tools/perf/util/trace-event.h` 20

- header: `struct trace_event`
- doc: ""

```
    17  struct evsel;
    18  struct tep_format_field;
    19  
    20  struct trace_event {
    21  	struct tep_handle	*pevent;
    22  	struct tep_plugin_list	*plugin_list;
```

### c/header/8: linux `drivers/gpu/drm/amd/pm/powerplay/inc/smu11_driver_if.h` 628

- header: `struct OverDriveTable_t`
- doc: ""

```
   625    uint32_t     MmHubPadding[8];
   626  } DriverSmuConfig_t;
   627  
   628  typedef struct {
   629  
   630    uint16_t      GfxclkFmin;
```

### c/header/9: linux `drivers/platform/x86/dell/dell-smbios.h` 96

- header: `function init_dell_smbios_wmi`
- doc: "CONFIG_DELL_SMBIOS_WMI"

```
    93  int init_dell_smbios_wmi(void);
    94  void exit_dell_smbios_wmi(void);
    95  #else /* CONFIG_DELL_SMBIOS_WMI */
    96  static inline int init_dell_smbios_wmi(void)
    97  {
    98  	return -ENODEV;
```

### c/header/10: linux `drivers/net/ethernet/mellanox/mlx5/core/steering/hws/cmd.c` 465

- header: `function hws_cmd_stc_modify_set_stc_param`
- doc: ""

```
   462  	hws_cmd_general_obj_destroy(mdev, MLX5_OBJ_TYPE_STC, stc_id);
   463  }
   464  
   465  static int
   466  hws_cmd_stc_modify_set_stc_param(struct mlx5_core_dev *mdev,
   467  				 struct mlx5hws_cmd_stc_modify_attr *stc_attr,
```

## java: headers (93815 in all)

### java/header/1: keycloak `testsuite/integration-arquillian/tests/base/src/test/java/org/keycloak/testsuite/broker/KcSamlBrokerArtifactBindingTest.java` 60

- header: `method testLoginWithSignatureAtAssertionLevelSuccess void ()`
- doc: ""

```
    57                  false, false);
    58      }
    59  
    60      @Test
    61      public void testLoginWithSignatureAtAssertionLevelSuccess() throws Exception {
    62          // success with signature at assertion level
```

### java/header/2: keycloak `model/jpa/src/main/java/org/keycloak/models/jpa/entities/ClientEntity.java` 53

- header: `class ClientEntity`
- doc: "@author <a href=\"mailto:bill@burkecentral.com\">Bill Burke</a>\n@version $Revision: 1 $"

```
    50   * @author <a href="mailto:bill@burkecentral.com">Bill Burke</a>
    51   * @version $Revision: 1 $
    52   */
    53  @Entity
    54  @Table(name="CLIENT", uniqueConstraints = {@UniqueConstraint(columnNames = {"REALM_ID", "CLIENT_ID"})})
    55  @NamedQueries({
```

### java/header/3: keycloak `quarkus/runtime/src/main/java/org/keycloak/quarkus/runtime/configuration/mappers/DatabasePropertyMappers.java` 524

- header: `method checkSettingsAndVendor boolean (Collection<Vendor> validForVendors, String timeoutProperty, String datasource, Vendor vendor, String db)`
- doc: ""

```
   521                  !Objects.equals(Database.getDriver(db, false).orElse(null), dbDriver);
   522      }
   523  
   524      private static boolean checkSettingsAndVendor(Collection<Vendor> validForVendors, String timeoutProperty, String datasource, Vendor vendor, String db) {
   525          if (!validForVendors.contains(vendor)) {
   526              // this jdbc property is not for this vendor
```

### java/header/4: keycloak `tests/base/src/test/java/org/keycloak/tests/admin/DeclarativeUserTest.java` 140

- header: `method testValidationUsingExistingAttributes void ()`
- doc: ""

```
   137          Assertions.assertNull(user1.getAttributes());
   138      }
   139  
   140      @Test
   141      public void testValidationUsingExistingAttributes() {
   142          setUserProfileConfiguration(managedRealm.admin(), "{\"attributes\": ["
```

### java/header/5: keycloak `tests/base/src/test/java/org/keycloak/tests/sessionlimits/KcSamlUserSessionLimitsBrokerTest.java` 66

- header: `method createSamlUserPropertyMapper ProtocolMapperRepresentation (
            String name, String userAttribute, String samlAttributeName, String friendlyName)`
- doc: ""

```
    63          Assertions.assertTrue(oauth.parseLoginResponse().isSuccess());
    64      }
    65  
    66      private static ProtocolMapperRepresentation createSamlUserPropertyMapper(
    67              String name, String userAttribute, String samlAttributeName, String friendlyName) {
    68          ProtocolMapperRepresentation mapper = new ProtocolMapperRepresentation();
```

### java/header/6: keycloak `server-spi/src/main/java/org/keycloak/models/AuthenticationExecutionModel.java` 144

- header: `method valueOfInteger Requirement (Integer id)`
- doc: ""

```
   141              return stableIndex;
   142          }
   143  
   144          public static Requirement valueOfInteger(Integer id) {
   145              return id == null ? null : BY_ID.get(id);
   146          }
```

### java/header/7: keycloak `services/src/main/java/org/keycloak/protocol/oidc/refresh/RefreshTokenProvider.java` 72

- header: `method getProviderId String ()`
- doc: "Returns the provider ID that this instance uses to identify issued refresh tokens.\nUsed for event detail emission during initial issuance\nand refresh-token processing when the token does not carry a provider claim"

```
    69       * Used for event detail emission during initial issuance
    70       * and refresh-token processing when the token does not carry a provider claim
    71       */
    72      String getProviderId();
    73  
    74      @Override
```

### java/header/8: keycloak `tests/base/src/test/java/org/keycloak/tests/events/EventRunOnServerHelper.java` 127

- header: `method clearExpiredEvents void ()`
- doc: ""

```
   124          });
   125      }
   126  
   127      public void clearExpiredEvents() {
   128          runOnServer.run(session -> {
   129              EventStoreProvider eventStore = session.getProvider(EventStoreProvider.class);
```

### java/header/9: keycloak `testsuite/integration-arquillian/tests/base/src/test/java/org/keycloak/testsuite/federation/ldap/noimport/LDAPRoleMappingsNoImportTest.java` 191

- header: `method test02WriteMappings void ()`
- doc: ""

```
   188      }
   189  
   190  
   191      @Test
   192      public void test02WriteMappings() {
   193          testingClient.server().run(session -> {
```

### java/header/10: keycloak `services/src/main/java/org/keycloak/protocol/oauth2/cimd/clientpolicy/executor/AbstractClientIdMetadataDocumentExecutor.java` 218

- header: `method OIDCClientRepresentationWithCacheControl(OIDCClientRepresentation oidcClientRepresentation,
                                                        ClientMetadataCacheControl clientMetadataCacheControl)`
- doc: ""

```
   215          private final OIDCClientRepresentation oidcClientRepresentation;
   216          private final ClientMetadataCacheControl clientMetadataCacheControl;
   217  
   218          public OIDCClientRepresentationWithCacheControl(OIDCClientRepresentation oidcClientRepresentation,
   219                                                          ClientMetadataCacheControl clientMetadataCacheControl) {
   220              this.oidcClientRepresentation = oidcClientRepresentation;
```

## javascript: headers (1130 in all)

### javascript/header/1: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.min.js` 6

- header: `function _default(u)`
- doc: ""

```
     3   * XRegExp Unicode Base 5.1.1
     4   * <xregexp.com>
     5   * Steven Levithan (c) 2008-present MIT License
     6   */t.default=function _default(u){var d={},t={},a=u._dec,c=u._hex,i=u._pad4;function normalize(u){return u.replace(/[- _]+/g,"").toLowerCase()}function charCode(u){var d=/^\\[xu](.+)/.exec(u);return d
     7  /*!
     8   * XRegExp Unicode Categories 5.1.1
```

### javascript/header/2: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.min.js` 17

- header: `function sort(u)`
- doc: ""

```
    14   * XRegExp 5.1.1
    15   * <xregexp.com>
    16   * Steven Levithan (c) 2007-present MIT License
    17   */var B={astral:!1,namespacing:!0},w={},j={},k={},S=[],O="default",R="class",_={default:/\\(?:0(?:[0-3][0-7]{0,2}|[4-7][0-7]?)?|[1-9]\d*|x[\dA-Fa-f]{2}|u(?:[\dA-Fa-f]{4}|{[\dA-Fa-f]+})|c[A-Za-z]|[\s\
```

### javascript/header/3: excalidraw `scripts/build-version.js` 8

- header: `function versionDate(date)`
- doc: ""

```
     5  const versionFile = path.join("build", "version.json");
     6  const indexFile = path.join("build", "index.html");
     7  
     8  const versionDate = (date) => date.toISOString().replace(".000", "");
     9  
    10  const commitHash = () => {
```

### javascript/header/4: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.min.js` 1

- header: `function o(c,i)`
- doc: ""

```
     1  !function(u){if("object"==typeof exports&&"undefined"!=typeof module)module.exports=u();else if("function"==typeof define&&define.amd)define([],u);else{("undefined"!=typeof window?window:"undefined"!=
     2  /*!
     3   * XRegExp Unicode Base 5.1.1
```

### javascript/header/5: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.min.js` 17

- header: `function NullProtoObjectViaActiveX(u)`
- doc: ""

```
    14   * XRegExp 5.1.1
    15   * <xregexp.com>
    16   * Steven Levithan (c) 2007-present MIT License
    17   */var B={astral:!1,namespacing:!0},w={},j={},k={},S=[],O="default",R="class",_={default:/\\(?:0(?:[0-3][0-7]{0,2}|[4-7][0-7]?)?|[1-9]\d*|x[\dA-Fa-f]{2}|u(?:[\dA-Fa-f]{4}|{[\dA-Fa-f]+})|c[A-Za-z]|[\s\
```

### javascript/header/6: django `django/contrib/admin/static/admin/js/vendor/select2/select2.full.min.js` 2

- header: `function i(e)`
- doc: ""

```
     1  /*! Select2 4.0.13 | https://github.com/select2/select2/blob/master/LICENSE.md */
     2  !function(n){"function"==typeof define&&define.amd?define(["jquery"],n):"object"==typeof module&&module.exports?module.exports=function(e,t){return void 0===t&&(t="undefined"!=typeof window?require("j
```

### javascript/header/7: django `django/contrib/admin/static/admin/js/SelectFilter2.js` 266

- header: `function refresh_filtered_selects(field_id)`
- doc: ""

```
   263                  selector.className += ' selector-chosen--with-filtered';
   264              }
   265          },
   266          refresh_filtered_selects: function(field_id) {
   267              SelectBox.filter(field_id + '_from', document.getElementById(field_id + "_input").value);
   268              SelectBox.filter(field_id + '_to', document.getElementById(field_id + "_selected_input").value);
```

### javascript/header/8: django `django/contrib/admin/static/admin/js/vendor/jquery/jquery.js` 9300

- header: `function setRequestHeader( name, value )`
- doc: ""

```
  9297  				},
  9298  
  9299  				// Caches the header
  9300  				setRequestHeader: function( name, value ) {
  9301  					if ( completed == null ) {
  9302  						name = requestHeadersNames[ name.toLowerCase() ] =
```

### javascript/header/9: django `django/contrib/admin/static/admin/js/vendor/xregexp/xregexp.js` 1865

- header: `method union(patterns, flags, options)`
- doc: "Returns an XRegExp object that is the union of the given patterns. Patterns can be provided as\nregex objects or strings. Metacharacters are escaped in patterns provided as strings.\nBackreferences in provided regex objects are automatically renumbered to work correctly within\nthe larger combined patt"

```
  1862   */
  1863  
  1864  
  1865  XRegExp.union = function (patterns, flags, options) {
  1866    options = options || {};
  1867    var conjunction = options.conjunction || 'or';
```

### javascript/header/10: django `django/contrib/admin/static/admin/js/vendor/jquery/jquery.js` 9535

- header: `function done( status, nativeStatusText, responses, headers )`
- doc: "Callback for when everything is done"

```
  9532  		}
  9533  
  9534  		// Callback for when everything is done
  9535  		function done( status, nativeStatusText, responses, headers ) {
  9536  			var isSuccess, success, error, response, modified,
  9537  				statusText = nativeStatusText;
```

## python: headers (47908 in all)

### python/header/1: django `django/contrib/gis/db/models/lookups.py` 296

- header: `class WithinLookup`
- doc: ""

```
   293  
   294  
   295  @BaseSpatialField.register_lookup
   296  class WithinLookup(GISLookup):
   297      lookup_name = "within"
   298  
```

### python/header/2: linux `tools/net/ynl/pyynl/ynl_gen_c.py` 459

- header: `method _attr_typol(self)`
- doc: ""

```
   456      def arg_member(self, ri):
   457          return []
   458  
   459      def _attr_typol(self):
   460          return '.type = YNL_PT_FLAG, '
   461  
```

### python/header/3: django `tests/forms_tests/widget_tests/test_textarea.py` 67

- header: `class TestForm`
- doc: ""

```
    64          )
    65  
    66      def test_fieldset(self):
    67          class TestForm(Form):
    68              template_name = "forms_tests/use_fieldset.html"
    69              field = CharField(widget=self.widget)
```

### python/header/4: django `tests/template_tests/syntax_tests/i18n/test_blocktranslate.py` 136

- header: `method test_i18n08(self)`
- doc: "translation of plural form"

```
   133              "{{ counter }} plural{% endblocktranslate %}"
   134          }
   135      )
   136      def test_i18n08(self):
   137          """translation of plural form"""
   138          output = self.engine.render_to_string("i18n08", {"number": 2})
```

### python/header/5: django `django/contrib/staticfiles/handlers.py` 29

- header: `method get_base_url(self)`
- doc: ""

```
    26          # them for self.
    27          pass
    28  
    29      def get_base_url(self):
    30          utils.check_settings()
    31          return settings.STATIC_URL
```

### python/header/6: django `tests/invalid_models_tests/test_ordinary_fields.py` 69

- header: `class Model`
- doc: ""

```
    66  @isolate_apps("invalid_models_tests")
    67  class BinaryFieldTests(SimpleTestCase):
    68      def test_valid_default_value(self):
    69          class Model(models.Model):
    70              field1 = models.BinaryField(default=b"test")
    71              field2 = models.BinaryField(default=None)
```

### python/header/7: django `tests/template_tests/syntax_tests/i18n/test_translate.py` 99

- header: `method test_i18n24(self)`
- doc: ""

```
    96          self.assertEqual(output, "nicht gefunden")
    97  
    98      @setup({"i18n24": "{% load i18n %}{% translate 'Page not found'|upper %}"})
    99      def test_i18n24(self):
   100          with translation.override("de"):
   101              output = self.engine.render_to_string("i18n24")
```

### python/header/8: django `tests/migrations/test_state.py` 28

- header: `class StateTests`
- doc: "Tests state construction, rendering and modification by operations."

```
    25  )
    26  
    27  
    28  class StateTests(SimpleTestCase):
    29      """
    30      Tests state construction, rendering and modification by operations.
```

### python/header/9: django `tests/sites_tests/tests.py` 309

- header: `method test_signal(self)`
- doc: "#23641 - Sending the ``post_migrate`` signal triggers creation of the\ndefault site."

```
   306          create_default_site(self.app_config, verbosity=0)
   307          Site(domain="example2.com", name="example2.com").save()
   308  
   309      def test_signal(self):
   310          """
   311          #23641 - Sending the ``post_migrate`` signal triggers creation of the
```

### python/header/10: django `django/utils/text.py` 157

- header: `class TruncateCharsHTMLParser`
- doc: ""

```
   154              self.reset()
   155  
   156  
   157  class TruncateCharsHTMLParser(TruncateHTMLParser):
   158      def __init__(self, *, length, replacement, convert_charrefs=True):
   159          self.length = length
```

## rust: headers (7047 in all)

### rust/header/1: ripgrep `tests/util.rs` 300

- header: `method stdout(&mut self) -> String`
- doc: "Runs and captures the stdout of the given command."

```
   297      }
   298  
   299      /// Runs and captures the stdout of the given command.
   300      pub fn stdout(&mut self) -> String {
   301          let o = self.output();
   302          String::from_utf8_lossy(&o.stdout).into_owned()
```

### rust/header/2: ripgrep `crates/core/flags/mod.rs` 82

- header: `method name_short(&self) -> Option<u8>`
- doc: "A short single byte name for this flag. This returns `None` by default,\nwhich signifies that the flag has no short name.\n\nThe byte returned must be an ASCII codepoint that is a `.` or is\nalpha-numeric."

```
    79      ///
    80      /// The byte returned must be an ASCII codepoint that is a `.` or is
    81      /// alpha-numeric.
    82      fn name_short(&self) -> Option<u8> {
    83          None
    84      }
```

### rust/header/3: linux `rust/kernel/alloc/kvec.rs` 152

- header: `type_alias KVVec`
- doc: "Type alias for [`Vec`] with a [`KVmalloc`] allocator.\n\nExamples\n\n```\nlet mut v = KVVec::new();\nv.push(1, GFP_KERNEL)?;\nassert_eq!(&v, &[1]);\n\nOk::<(), Error>(())\n```"

```
   149  ///
   150  /// # Ok::<(), Error>(())
   151  /// ```
   152  pub type KVVec<T> = Vec<T, KVmalloc>;
   153  
   154  // SAFETY: `Vec` is `Send` if `T` is `Send` because `Vec` owns its elements.
```

### rust/header/4: ripgrep `crates/searcher/src/searcher/core.rs` 201

- header: `method detect_binary(
        &mut self,
        buf: &[u8],
        range: &Range,
    ) -> Result<bool, S::Error>`
- doc: ""

```
   198          consumed
   199      }
   200  
   201      pub(crate) fn detect_binary(
   202          &mut self,
   203          buf: &[u8],
```

### rust/header/5: linux `drivers/gpu/nova-core/fb/hal/ga100.rs` 13

- header: `function read_sysmem_flush_page_ga100(bar: &Bar0) -> u64`
- doc: ""

```
    10  
    11  use super::tu102::FLUSH_SYSMEM_ADDR_SHIFT;
    12  
    13  pub(super) fn read_sysmem_flush_page_ga100(bar: &Bar0) -> u64 {
    14      u64::from(regs::NV_PFB_NISO_FLUSH_SYSMEM_ADDR::read(bar).adr_39_08()) << FLUSH_SYSMEM_ADDR_SHIFT
    15          | u64::from(regs::NV_PFB_NISO_FLUSH_SYSMEM_ADDR_HI::read(bar).adr_63_40())
```

### rust/header/6: linux `rust/kernel/sync/poll.rs` 95

- header: `method drop(self: Pin<&mut Self>)`
- doc: ""

```
    92  #[pinned_drop]
    93  impl PinnedDrop for PollCondVar {
    94      #[inline]
    95      fn drop(self: Pin<&mut Self>) {
    96          // Clear anything registered using `register_wait`.
    97          //
```

### rust/header/7: linux `rust/pin-init/src/lib.rs` 1407

- header: `method __pinned_init(self, slot: *mut T) -> Result<(), Infallible>`
- doc: ""

```
  1404  // SAFETY: the `__pinned_init` function always returns `Ok(())` and initializes every field of
  1405  // `slot`. Additionally, all pinning invariants of `T` are upheld.
  1406  unsafe impl<T> PinInit<T> for T {
  1407      unsafe fn __pinned_init(self, slot: *mut T) -> Result<(), Infallible> {
  1408          // SAFETY: `slot` is valid for writes by the safety requirements of this function.
  1409          unsafe { slot.write(self) };
```

### rust/header/8: ripgrep `crates/ignore/src/walk.rs` 2801

- header: `function same_file_system()`
- doc: ""

```
  2798      // with that.
  2799      #[test]
  2800      #[cfg(target_os = "linux")]
  2801      fn same_file_system() {
  2802          use super::device_num;
  2803  
```

### rust/header/9: ripgrep `crates/printer/src/util.rs` 662

- header: `function escape_path()`
- doc: ""

```
   659      }
   660  
   661      #[test]
   662      fn escape_path() {
   663          let esc = |b: &[u8]| {
   664              let path = b.to_path().unwrap();
```

### rust/header/10: linux `rust/kernel/mm.rs` 48

- header: `struct Mm`
- doc: ""

```
    45  ///
    46  /// [`mmget_not_zero`]: Mm::mmget_not_zero
    47  #[repr(transparent)]
    48  pub struct Mm {
    49      mm: Opaque<bindings::mm_struct>,
    50  }
```

## tsx: headers (3745 in all)

### tsx/header/1: keycloak `js/apps/admin-ui/src/groups/GroupTable.tsx` 22

- header: `type_alias GroupTableProps`
- doc: ""

```
    19  import { getLastId } from "./groupIdUtils";
    20  import { useGroupResource } from "../context/group-resource/GroupResourceContext";
    21  
    22  type GroupTableProps = {
    23    refresh: () => void;
    24  };
```

### tsx/header/2: excalidraw `packages/excalidraw/components/Sidebar/SidebarHeader.tsx` 10

- header: `function SidebarHeader({
  children,
  className,
}: {
  children?: React.ReactNode;
  className?: string;
})`
- doc: ""

```
     7  import { Tooltip } from "../Tooltip";
     8  import { Button } from "../Button";
     9  
    10  export const SidebarHeader = ({
    11    children,
    12    className,
```

### tsx/header/3: keycloak `js/apps/account-ui/src/account-security/AccountRow.tsx` 41

- header: `function unLink(account: LinkedAccountRepresentation)`
- doc: ""

```
    38    const { login } = context.keycloak;
    39    const { addAlert, addError } = useAccountAlerts();
    40  
    41    const unLink = async (account: LinkedAccountRepresentation) => {
    42      try {
    43        await unLinkAccount(context, account);
```

### tsx/header/4: keycloak `js/apps/admin-ui/src/components/keycloak-card/ClickableCard.tsx` 8

- header: `function ClickableCard({
  onClick,
  children,
  ...rest
}: ClickableCardProps)`
- doc: ""

```
     5    onClick: () => void;
     6  };
     7  
     8  export const ClickableCard = ({
     9    onClick,
    10    children,
```

### tsx/header/5: keycloak `js/apps/admin-ui/src/components/keycloak-card/ClickableCard.tsx` 4

- header: `type_alias ClickableCardProps`
- doc: ""

```
     1  import { KeyboardEvent, useId } from "react";
     2  import { Card, CardHeader, CardProps } from "@patternfly/react-core";
     3  
     4  type ClickableCardProps = Omit<CardProps, "onClick"> & {
     5    onClick: () => void;
     6  };
```

### tsx/header/6: excalidraw `packages/excalidraw/components/ImageExportDialog.tsx` 369

- header: `function ImageExportDialog({
  elements,
  appState,
  files,
  actionManager,
  onExportImage,
  onCloseRequest,
  name,
}: {
  appState: UIAppState;
  elements: readonly NonDeletedExcalidrawElement[];
  files: BinaryFiles;
  actionManager: ActionManager;
  onExportImage: AppClassProperties["onExportImage"];
  onCloseRequest: () => void;
  name: string;
})`
- doc: ""

```
   366    );
   367  };
   368  
   369  export const ImageExportDialog = ({
   370    elements,
   371    appState,
```

### tsx/header/7: keycloak `js/apps/admin-ui/src/components/dynamic/MapComponent.tsx` 68

- header: `function remove(index: number)`
- doc: ""

```
    65      return newMap;
    66    };
    67  
    68    const remove = (index: number) => {
    69      const value = [...map.slice(0, index), ...map.slice(index + 1)];
    70      setMap(value);
```

### tsx/header/8: keycloak `js/apps/account-ui/src/resources/EditTheResource.tsx` 25

- header: `function EditTheResource({
  resource,
  permissions,
  onClose,
}: EditTheResourceProps)`
- doc: ""

```
    22    permissions: Permission[];
    23  };
    24  
    25  export const EditTheResource = ({
    26    resource,
    27    permissions,
```

### tsx/header/9: keycloak `js/apps/admin-ui/src/authentication/form/CreateFlow.tsx` 26

- header: `function CreateFlow()`
- doc: ""

```
    23  
    24  const TYPES = ["basic-flow", "client-flow"] as const;
    25  
    26  export default function CreateFlow() {
    27    const { adminClient } = useAdminClient();
    28  
```

### tsx/header/10: excalidraw `packages/excalidraw/components/App.tsx` 836

- header: `method cacheEmbeddableRef(
    element: ExcalidrawIframeLikeElement,
    ref: HTMLIFrameElement | null,
  )`
- doc: ""

```
   833      }
   834    }
   835  
   836    private cacheEmbeddableRef(
   837      element: ExcalidrawIframeLikeElement,
   838      ref: HTMLIFrameElement | null,
```

## typescript: headers (3583 in all)

### typescript/header/1: keycloak `js/apps/admin-ui/test/realm-settings/sessions.ts` 4

- header: `function goToSessionsTab(page: Page)`
- doc: ""

```
     1  import { expect, type Page } from "@playwright/test";
     2  import { changeTimeUnit } from "../utils/form.ts";
     3  
     4  export async function goToSessionsTab(page: Page) {
     5    await page.getByTestId("rs-sessions-tab").click();
     6  }
```

### typescript/header/2: keycloak `js/apps/admin-ui/test/masthead/main.ts` 41

- header: `function toggleMobileViewHelp(page: Page)`
- doc: ""

```
    38    await (await getUserDropdown(page)).click();
    39  }
    40  
    41  export async function toggleMobileViewHelp(page: Page) {
    42    await page.getByRole("menuitem", { name: "Help on" }).click();
    43  }
```

### typescript/header/3: keycloak `js/apps/admin-ui/test/clients/advanced.ts` 15

- header: `function expandClusterNode(page: Page)`
- doc: ""

```
    12    await page.getByTestId("advancedTab").click();
    13  }
    14  
    15  export async function expandClusterNode(page: Page) {
    16    await page.getByRole("button", { name: "Registered cluster nodes" }).click();
    17  }
```

### typescript/header/4: excalidraw `packages/excalidraw/subset/woff2/woff2-bindings.ts` 1909

- header: `function getInheritedInstance(class_, ptr)`
- doc: ""

```
  1906        }
  1907        return ptr;
  1908      }
  1909      function getInheritedInstance(class_, ptr) {
  1910        ptr = getBasestPointer(class_, ptr);
  1911        return registeredInstances[ptr];
```

### typescript/header/5: keycloak `js/apps/admin-ui/src/context/auth/admin-ui-endpoint.ts` 44

- header: `function postAdminUI(
  adminClient: KeycloakAdminClient,
  endpoint: string,
  body: unknown,
): Promise<T | undefined>`
- doc: ""

```
    41    return await response.json();
    42  }
    43  
    44  export async function postAdminUI<T>(
    45    adminClient: KeycloakAdminClient,
    46    endpoint: string,
```

### typescript/header/6: excalidraw `packages/excalidraw/element/types.ts` 189

- header: `type_alias ExcalidrawRectanguloidElement`
- doc: ""

```
   186    | ExcalidrawDiamondElement
   187    | ExcalidrawEllipseElement;
   188  
   189  export type ExcalidrawRectanguloidElement =
   190    | ExcalidrawRectangleElement
   191    | ExcalidrawImageElement
```

### typescript/header/7: excalidraw `packages/excalidraw/utils.ts` 1126

- header: `function normalizeEOL(str: string)`
- doc: ""

```
  1123    return result;
  1124  }
  1125  
  1126  export const normalizeEOL = (str: string) => {
  1127    return str.replace(/\r?\n|\r/g, "\n");
  1128  };
```

### typescript/header/8: keycloak `js/libs/keycloak-admin-client/src/defs/ssfRepresentation.ts` 1

- header: `interface SsfStreamDeliveryConfig`
- doc: ""

```
     1  export interface SsfStreamDeliveryConfig {
     2    method?: string;
     3    endpoint_url?: string;
```

### typescript/header/9: excalidraw `packages/excalidraw/tests/helpers/ui.ts` 298

- header: `method clickOn(element: ExcalidrawElement)`
- doc: ""

```
   295      this.reset();
   296    }
   297  
   298    clickOn(element: ExcalidrawElement) {
   299      this.reset();
   300      this.click(...getElementPointForSelection(element));
```

### typescript/header/10: keycloak `js/apps/admin-ui/test/realm-settings/sessions.ts` 16

- header: `function getSsoSessionIdleRememberMe(page: Page)`
- doc: ""

```
    13    return page.getByTestId("sso-session-max-input");
    14  }
    15  
    16  function getSsoSessionIdleRememberMe(page: Page) {
    17    return page.getByTestId("sso-session-idle-remember-me-input");
    18  }
```
