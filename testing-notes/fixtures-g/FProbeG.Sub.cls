Class FProbeG.Sub Extends FProbeG.Target
{

ClassMethod Edge10() As %Integer
{
    // E10: instance-variable call site
    set o = ..%New()
    set r = o.TgtMethod(10)
    Quit r
}

}
