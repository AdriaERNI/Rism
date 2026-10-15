Class FProbeG.Caller
{

ClassMethod Edge1() As %Status
{
    // E1: plain ##class literal call site
    set t = ##class(FProbeG.Target).TgtMethod(1)
    quit $$$OK
}

ClassMethod Edge4(obj As %RegisteredObject) As %Status
{
    // E4: dynamic method name in a string
    set r = $zobjmethod(obj, "TgtMethod", 4)
    quit $$$OK
}

ClassMethod Edge5() As %Status
{
    // E5: class name assembled from a string, eval'd
    set cls = "FProbeG"_".Target"
    set cmd = "s x=##class("_cls_").TgtMethod(5)"
    do $ZF(-4,1,cmd)
    quit $$$OK
}

}
