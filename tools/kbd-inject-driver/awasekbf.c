// ADR-255 の実機相当の検証用(使い捨て、CI の windows-latest 専用): Keyboard クラスの下位(ポートドライバの上)に載る
// WDM フィルタドライバ。kbdclass が接続時に渡すクラスサービスコールバック(IOCTL_INTERNAL_KEYBOARD_CONNECT)を控え、
// 制御デバイス \\.\AwaseKbInject への IOCTL で受けたスキャンコードを、そのコールバックへ流す(物理キーと同じ経路。
// LLKHF_INJECTED は立たず、dwExtraInfo も 0)。テスト署名の CI ランナー以外では読み込まれない・読み込んではならない。
#include <ntddk.h>
#include <kbdmou.h>
#include <ntddkbd.h>

#define AWASE_IOCTL_INJECT CTL_CODE(FILE_DEVICE_KEYBOARD, 0x800, METHOD_BUFFERED, FILE_ANY_ACCESS)
#define AWASE_IOCTL_STATUS CTL_CODE(FILE_DEVICE_KEYBOARD, 0x801, METHOD_BUFFERED, FILE_ANY_ACCESS)

typedef struct _EXT {
    PDEVICE_OBJECT Lower;
} EXT, *PEXT;

typedef struct _KEY_CMD {
    USHORT MakeCode;
    USHORT Flags;   // KEY_MAKE=0, KEY_BREAK=1, KEY_E0=2, KEY_E1=4
} KEY_CMD;

static CONNECT_DATA g_conn;
static volatile LONG g_seenQuery, g_seenAny, g_modified;
static ULONG g_origType, g_origSubtype;
static ULONG g_jis;   // Parameters\JisType=1 のとき、キーボードの種類を 106/109 JIS(type 7, subtype 2)と申告し直す。既定は 0(何もしない)。
static PDEVICE_OBJECT g_ctl;

static NTSTATUS Pass(PDEVICE_OBJECT dev, PIRP irp)
{
    PEXT ext = (PEXT)dev->DeviceExtension;
    IoSkipCurrentIrpStackLocation(irp);
    return IoCallDriver(ext->Lower, irp);
}

static NTSTATUS CtlCreateClose(PDEVICE_OBJECT dev, PIRP irp)
{
    UNREFERENCED_PARAMETER(dev);
    irp->IoStatus.Status = STATUS_SUCCESS;
    irp->IoStatus.Information = 0;
    IoCompleteRequest(irp, IO_NO_INCREMENT);
    return STATUS_SUCCESS;
}


static NTSTATUS QueryAttrDone(PDEVICE_OBJECT dev, PIRP irp, PVOID ctx)
{
    UNREFERENCED_PARAMETER(dev);
    UNREFERENCED_PARAMETER(ctx);
    if (irp->PendingReturned) IoMarkIrpPending(irp);
    if (NT_SUCCESS(irp->IoStatus.Status) && irp->IoStatus.Information >= sizeof(KEYBOARD_ATTRIBUTES)) {
        PKEYBOARD_ATTRIBUTES a = (PKEYBOARD_ATTRIBUTES)irp->AssociatedIrp.SystemBuffer;
        if (a == NULL) {
            PIO_STACK_LOCATION cur = IoGetCurrentIrpStackLocation(irp);
            a = (PKEYBOARD_ATTRIBUTES)cur->Parameters.DeviceIoControl.Type3InputBuffer;
        }
        if (a != NULL) {
            g_origType = a->KeyboardIdentifier.Type; g_origSubtype = a->KeyboardIdentifier.Subtype;
            InterlockedIncrement(&g_modified);
            a->KeyboardIdentifier.Type = 7;
            a->KeyboardIdentifier.Subtype = 2;
            a->KeyboardMode = 1;
            a->NumberOfFunctionKeys = 12;
            a->NumberOfIndicators = 3;
            a->NumberOfKeysTotal = 106;
        }
    }
    return STATUS_CONTINUE_COMPLETION;
}

static NTSTATUS Dispatch(PDEVICE_OBJECT dev, PIRP irp)
{
    if (dev == g_ctl) {
        PIO_STACK_LOCATION sp = IoGetCurrentIrpStackLocation(irp);
        if (sp->MajorFunction == IRP_MJ_CREATE || sp->MajorFunction == IRP_MJ_CLOSE || sp->MajorFunction == IRP_MJ_CLEANUP)
            return CtlCreateClose(dev, irp);
        if (sp->MajorFunction == IRP_MJ_DEVICE_CONTROL &&
            sp->Parameters.DeviceIoControl.IoControlCode == AWASE_IOCTL_STATUS) {
            ULONG *o = (ULONG *)irp->AssociatedIrp.SystemBuffer;
            if (sp->Parameters.DeviceIoControl.OutputBufferLength >= 7 * sizeof(ULONG)) {
                o[0] = g_jis; o[1] = (ULONG)g_seenAny; o[2] = (ULONG)g_seenQuery; o[3] = (ULONG)g_modified; o[4] = g_origType; o[5] = g_origSubtype; o[6] = (g_conn.ClassService != NULL);
                irp->IoStatus.Status = STATUS_SUCCESS; irp->IoStatus.Information = 7 * sizeof(ULONG);
            } else { irp->IoStatus.Status = STATUS_BUFFER_TOO_SMALL; irp->IoStatus.Information = 0; }
            IoCompleteRequest(irp, IO_NO_INCREMENT);
            return irp->IoStatus.Status;
        }
        if (sp->MajorFunction == IRP_MJ_DEVICE_CONTROL &&
            sp->Parameters.DeviceIoControl.IoControlCode == AWASE_IOCTL_INJECT) {
            NTSTATUS st = STATUS_DEVICE_NOT_CONNECTED;
            ULONG inLen = sp->Parameters.DeviceIoControl.InputBufferLength;
            ULONG n = inLen / sizeof(KEY_CMD);
            KEY_CMD *cmds = (KEY_CMD *)irp->AssociatedIrp.SystemBuffer;
            if (g_conn.ClassService != NULL && n > 0 && n <= 64) {
                KEYBOARD_INPUT_DATA data[64];
                ULONG i, consumed = 0;
                KIRQL old;
                RtlZeroMemory(data, sizeof(data));
                for (i = 0; i < n; i++) {
                    data[i].UnitId = 0;
                    data[i].MakeCode = cmds[i].MakeCode;
                    data[i].Flags = cmds[i].Flags;
                    data[i].ExtraInformation = 0;
                }
                // クラスサービスコールバックは DISPATCH_LEVEL で呼ぶ決まり。
                KeRaiseIrql(DISPATCH_LEVEL, &old);
                ((PSERVICE_CALLBACK_ROUTINE)g_conn.ClassService)(g_conn.ClassDeviceObject, &data[0], &data[n], &consumed);
                KeLowerIrql(old);
                st = STATUS_SUCCESS;
                irp->IoStatus.Information = consumed * sizeof(KEYBOARD_INPUT_DATA);
            } else if (n == 0 || n > 64) {
                st = STATUS_INVALID_PARAMETER;
            }
            irp->IoStatus.Status = st;
            IoCompleteRequest(irp, IO_NO_INCREMENT);
            return st;
        }
        irp->IoStatus.Status = STATUS_INVALID_DEVICE_REQUEST;
        irp->IoStatus.Information = 0;
        IoCompleteRequest(irp, IO_NO_INCREMENT);
        return STATUS_INVALID_DEVICE_REQUEST;
    }
    {
        PIO_STACK_LOCATION spq = IoGetCurrentIrpStackLocation(irp);
        if (spq->MajorFunction == IRP_MJ_DEVICE_CONTROL) {
            InterlockedIncrement(&g_seenAny);
            if (spq->Parameters.DeviceIoControl.IoControlCode == IOCTL_KEYBOARD_QUERY_ATTRIBUTES) InterlockedIncrement(&g_seenQuery);
        }
    }
    if (g_jis) {
        PIO_STACK_LOCATION sp2 = IoGetCurrentIrpStackLocation(irp);
        if (sp2->MajorFunction == IRP_MJ_DEVICE_CONTROL &&
            sp2->Parameters.DeviceIoControl.IoControlCode == IOCTL_KEYBOARD_QUERY_ATTRIBUTES) {
            PEXT ext = (PEXT)dev->DeviceExtension;
            IoCopyCurrentIrpStackLocationToNext(irp);
            IoSetCompletionRoutine(irp, QueryAttrDone, NULL, TRUE, TRUE, TRUE);
            return IoCallDriver(ext->Lower, irp);
        }
    }
    return Pass(dev, irp);
}

static NTSTATUS InternalIoctl(PDEVICE_OBJECT dev, PIRP irp)
{
    PIO_STACK_LOCATION sp = IoGetCurrentIrpStackLocation(irp);
    if (dev == g_ctl) { irp->IoStatus.Status = STATUS_INVALID_DEVICE_REQUEST; irp->IoStatus.Information = 0; IoCompleteRequest(irp, IO_NO_INCREMENT); return STATUS_INVALID_DEVICE_REQUEST; }
    if (dev != g_ctl && sp->Parameters.DeviceIoControl.IoControlCode == IOCTL_INTERNAL_KEYBOARD_CONNECT) {
        PCONNECT_DATA cd = (PCONNECT_DATA)sp->Parameters.DeviceIoControl.Type3InputBuffer;
        if (cd != NULL && sp->Parameters.DeviceIoControl.InputBufferLength >= sizeof(CONNECT_DATA)) {
            g_conn = *cd;   // 元のコールバックを控えるだけ。経路は変えず、そのまま下へ流す。
        }
    }
    if (dev != g_ctl) {
        InterlockedIncrement(&g_seenAny);
        if (sp->Parameters.DeviceIoControl.IoControlCode == IOCTL_KEYBOARD_QUERY_ATTRIBUTES) {
            InterlockedIncrement(&g_seenQuery);
            if (g_jis) {
                PEXT ext = (PEXT)dev->DeviceExtension;
                IoCopyCurrentIrpStackLocationToNext(irp);
                IoSetCompletionRoutine(irp, QueryAttrDone, NULL, TRUE, TRUE, TRUE);
                return IoCallDriver(ext->Lower, irp);
            }
        }
    }
    return Pass(dev, irp);
}

static NTSTATUS PnpDispatch(PDEVICE_OBJECT dev, PIRP irp)
{
    PIO_STACK_LOCATION sp = IoGetCurrentIrpStackLocation(irp);
    PEXT ext = (PEXT)dev->DeviceExtension;
    if (sp->MinorFunction == IRP_MN_REMOVE_DEVICE) {
        NTSTATUS st;
        IoSkipCurrentIrpStackLocation(irp);
        st = IoCallDriver(ext->Lower, irp);
        RtlZeroMemory(&g_conn, sizeof(g_conn));
        IoDetachDevice(ext->Lower);
        IoDeleteDevice(dev);
        return st;
    }
    return Pass(dev, irp);
}

static NTSTATUS PowerDispatch(PDEVICE_OBJECT dev, PIRP irp)
{
    PEXT ext = (PEXT)dev->DeviceExtension;
    IoSkipCurrentIrpStackLocation(irp);
    return PoCallDriver(ext->Lower, irp);
}

static NTSTATUS AddDevice(PDRIVER_OBJECT drv, PDEVICE_OBJECT pdo)
{
    PDEVICE_OBJECT fdo;
    PEXT ext;
    NTSTATUS st = IoCreateDevice(drv, sizeof(EXT), NULL, FILE_DEVICE_KEYBOARD, 0, FALSE, &fdo);
    if (!NT_SUCCESS(st)) return st;
    ext = (PEXT)fdo->DeviceExtension;
    ext->Lower = IoAttachDeviceToDeviceStack(fdo, pdo);
    if (ext->Lower == NULL) { IoDeleteDevice(fdo); return STATUS_UNSUCCESSFUL; }
    fdo->Flags |= ext->Lower->Flags & (DO_BUFFERED_IO | DO_DIRECT_IO | DO_POWER_PAGABLE);
    fdo->Flags &= ~DO_DEVICE_INITIALIZING;
    return STATUS_SUCCESS;
}

static VOID Unload(PDRIVER_OBJECT drv)
{
    UNICODE_STRING link = RTL_CONSTANT_STRING(L"\\DosDevices\\AwaseKbInject");
    UNREFERENCED_PARAMETER(drv);
    if (g_ctl != NULL) { IoDeleteSymbolicLink(&link); IoDeleteDevice(g_ctl); g_ctl = NULL; }
}

NTSTATUS DriverEntry(PDRIVER_OBJECT drv, PUNICODE_STRING reg)
{
    UNICODE_STRING name = RTL_CONSTANT_STRING(L"\\Device\\AwaseKbInject");
    UNICODE_STRING link = RTL_CONSTANT_STRING(L"\\DosDevices\\AwaseKbInject");
    NTSTATUS st;
    ULONG i;
    {
        // HKLM\\SYSTEM\\CurrentControlSet\\Services\\awasekbf\\Parameters の JisType(DWORD)。
        RTL_QUERY_REGISTRY_TABLE q[2];
        ULONG jis = 0, def = 0;
        RtlZeroMemory(q, sizeof(q));
        q[0].Flags = RTL_QUERY_REGISTRY_DIRECT | RTL_QUERY_REGISTRY_SUBKEY;
        q[0].Name = L"Parameters";
        q[0].EntryContext = NULL;
        q[0].Flags = RTL_QUERY_REGISTRY_SUBKEY;
        q[1].Flags = RTL_QUERY_REGISTRY_DIRECT;
        q[1].Name = L"JisType";
        q[1].EntryContext = &jis;
        q[1].DefaultType = REG_DWORD;
        q[1].DefaultData = &def;
        q[1].DefaultLength = sizeof(ULONG);
        {
            RTL_QUERY_REGISTRY_TABLE t[3];
            RtlZeroMemory(t, sizeof(t));
            t[0].Flags = RTL_QUERY_REGISTRY_SUBKEY;
            t[0].Name = L"Parameters";
            t[1] = q[1];
            if (NT_SUCCESS(RtlQueryRegistryValues(RTL_REGISTRY_ABSOLUTE, reg->Buffer, t, NULL, NULL))) g_jis = jis;
        }
    }
    for (i = 0; i <= IRP_MJ_MAXIMUM_FUNCTION; i++) drv->MajorFunction[i] = Dispatch;
    drv->MajorFunction[IRP_MJ_INTERNAL_DEVICE_CONTROL] = InternalIoctl;
    drv->MajorFunction[IRP_MJ_PNP] = PnpDispatch;
    drv->MajorFunction[IRP_MJ_POWER] = PowerDispatch;
    drv->DriverExtension->AddDevice = AddDevice;
    drv->DriverUnload = Unload;
    st = IoCreateDevice(drv, 0, &name, FILE_DEVICE_KEYBOARD, FILE_DEVICE_SECURE_OPEN, FALSE, &g_ctl);
    if (NT_SUCCESS(st)) {
        g_ctl->Flags |= DO_BUFFERED_IO;
        IoCreateSymbolicLink(&link, &name);
        g_ctl->Flags &= ~DO_DEVICE_INITIALIZING;
    }
    return STATUS_SUCCESS;
}
