<?xml version="1.0" encoding="UTF-8"?>
<tileset version="1.10" tiledversion="1.10.2" name="pell" tilewidth="32" tileheight="48" tilecount="128" columns="16">
 <properties>
  <property name="hitbox_width" type="float" value="1.0"/>
  <property name="hitbox_height" type="float" value="1.25"/>
  <property name="scale" type="float" value="0.5"/>
 </properties>
 <image source="pell.png" width="512" height="384"/>
 <tile id="0">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="0"/>
  </properties>
  <animation>
   <frame tileid="0" duration="80"/>
   <frame tileid="1" duration="100"/>
   <frame tileid="2" duration="120"/>
   <frame tileid="3" duration="100"/>
  </animation>
 </tile>
 <tile id="2">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="Block01"/>
  </properties>
 </tile>
 <tile id="4">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="1"/>
  </properties>
  <animation>
   <frame tileid="4" duration="80"/>
   <frame tileid="5" duration="100"/>
   <frame tileid="6" duration="120"/>
   <frame tileid="7" duration="100"/>
  </animation>
 </tile>
 <tile id="6">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="Block01"/>
  </properties>
 </tile>
 <tile id="8">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="2"/>
  </properties>
  <animation>
   <frame tileid="8" duration="80"/>
   <frame tileid="9" duration="100"/>
   <frame tileid="10" duration="120"/>
   <frame tileid="11" duration="100"/>
  </animation>
 </tile>
 <tile id="10">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="Block01"/>
  </properties>
 </tile>
 <tile id="12">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="3"/>
  </properties>
  <animation>
   <frame tileid="12" duration="80"/>
   <frame tileid="13" duration="100"/>
   <frame tileid="14" duration="120"/>
   <frame tileid="15" duration="100"/>
  </animation>
 </tile>
 <tile id="14">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="Block01"/>
  </properties>
 </tile>
 <tile id="16">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="4"/>
  </properties>
  <animation>
   <frame tileid="16" duration="80"/>
   <frame tileid="17" duration="100"/>
   <frame tileid="18" duration="120"/>
   <frame tileid="19" duration="100"/>
  </animation>
 </tile>
 <tile id="18">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="Block01"/>
  </properties>
 </tile>
 <tile id="20">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="5"/>
  </properties>
  <animation>
   <frame tileid="20" duration="80"/>
   <frame tileid="21" duration="100"/>
   <frame tileid="22" duration="120"/>
   <frame tileid="23" duration="100"/>
  </animation>
 </tile>
 <tile id="22">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="Block01"/>
  </properties>
 </tile>
 <tile id="24">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="6"/>
  </properties>
  <animation>
   <frame tileid="24" duration="80"/>
   <frame tileid="25" duration="100"/>
   <frame tileid="26" duration="120"/>
   <frame tileid="27" duration="100"/>
  </animation>
 </tile>
 <tile id="26">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="Block01"/>
  </properties>
 </tile>
 <tile id="28">
  <properties>
   <property name="action" value="attack"/>
   <property name="dir" type="int" value="7"/>
  </properties>
  <animation>
   <frame tileid="28" duration="80"/>
   <frame tileid="29" duration="100"/>
   <frame tileid="30" duration="120"/>
   <frame tileid="31" duration="100"/>
  </animation>
 </tile>
 <tile id="30">
  <properties>
   <property name="apex" type="bool" value="true"/>
   <property name="sfx" value="Block01"/>
  </properties>
 </tile>
 <tile id="32">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="0"/>
   <property name="sfx" value="Death01"/>
  </properties>
  <animation>
   <frame tileid="32" duration="90"/>
   <frame tileid="33" duration="90"/>
   <frame tileid="34" duration="400"/>
  </animation>
 </tile>
 <tile id="35">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="1"/>
   <property name="sfx" value="Death01"/>
  </properties>
  <animation>
   <frame tileid="35" duration="90"/>
   <frame tileid="36" duration="90"/>
   <frame tileid="37" duration="400"/>
  </animation>
 </tile>
 <tile id="38">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="2"/>
   <property name="sfx" value="Death01"/>
  </properties>
  <animation>
   <frame tileid="38" duration="90"/>
   <frame tileid="39" duration="90"/>
   <frame tileid="40" duration="400"/>
  </animation>
 </tile>
 <tile id="41">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="3"/>
   <property name="sfx" value="Death01"/>
  </properties>
  <animation>
   <frame tileid="41" duration="90"/>
   <frame tileid="42" duration="90"/>
   <frame tileid="43" duration="400"/>
  </animation>
 </tile>
 <tile id="44">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="4"/>
   <property name="sfx" value="Death01"/>
  </properties>
  <animation>
   <frame tileid="44" duration="90"/>
   <frame tileid="45" duration="90"/>
   <frame tileid="46" duration="400"/>
  </animation>
 </tile>
 <tile id="47">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="5"/>
   <property name="sfx" value="Death01"/>
  </properties>
  <animation>
   <frame tileid="47" duration="90"/>
   <frame tileid="48" duration="90"/>
   <frame tileid="49" duration="400"/>
  </animation>
 </tile>
 <tile id="50">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="6"/>
   <property name="sfx" value="Death01"/>
  </properties>
  <animation>
   <frame tileid="50" duration="90"/>
   <frame tileid="51" duration="90"/>
   <frame tileid="52" duration="400"/>
  </animation>
 </tile>
 <tile id="53">
  <properties>
   <property name="action" value="death"/>
   <property name="dir" type="int" value="7"/>
   <property name="sfx" value="Death01"/>
  </properties>
  <animation>
   <frame tileid="53" duration="90"/>
   <frame tileid="54" duration="90"/>
   <frame tileid="55" duration="400"/>
  </animation>
 </tile>
 <tile id="56">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="0"/>
  </properties>
  <animation>
   <frame tileid="56" duration="1000"/>
  </animation>
 </tile>
 <tile id="57">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="1"/>
  </properties>
  <animation>
   <frame tileid="57" duration="1000"/>
  </animation>
 </tile>
 <tile id="58">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="2"/>
  </properties>
  <animation>
   <frame tileid="58" duration="1000"/>
  </animation>
 </tile>
 <tile id="59">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="3"/>
  </properties>
  <animation>
   <frame tileid="59" duration="1000"/>
  </animation>
 </tile>
 <tile id="60">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="4"/>
  </properties>
  <animation>
   <frame tileid="60" duration="1000"/>
  </animation>
 </tile>
 <tile id="61">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="5"/>
  </properties>
  <animation>
   <frame tileid="61" duration="1000"/>
  </animation>
 </tile>
 <tile id="62">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="6"/>
  </properties>
  <animation>
   <frame tileid="62" duration="1000"/>
  </animation>
 </tile>
 <tile id="63">
  <properties>
   <property name="action" value="idle"/>
   <property name="dir" type="int" value="7"/>
  </properties>
  <animation>
   <frame tileid="63" duration="1000"/>
  </animation>
 </tile>
 <tile id="64">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="0"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="64" duration="100"/>
   <frame tileid="65" duration="100"/>
   <frame tileid="66" duration="100"/>
   <frame tileid="67" duration="100"/>
  </animation>
 </tile>
 <tile id="66">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="68">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="1"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="68" duration="100"/>
   <frame tileid="69" duration="100"/>
   <frame tileid="70" duration="100"/>
   <frame tileid="71" duration="100"/>
  </animation>
 </tile>
 <tile id="70">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="72">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="2"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="72" duration="100"/>
   <frame tileid="73" duration="100"/>
   <frame tileid="74" duration="100"/>
   <frame tileid="75" duration="100"/>
  </animation>
 </tile>
 <tile id="74">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="76">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="3"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="76" duration="100"/>
   <frame tileid="77" duration="100"/>
   <frame tileid="78" duration="100"/>
   <frame tileid="79" duration="100"/>
  </animation>
 </tile>
 <tile id="78">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="80">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="4"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="80" duration="100"/>
   <frame tileid="81" duration="100"/>
   <frame tileid="82" duration="100"/>
   <frame tileid="83" duration="100"/>
  </animation>
 </tile>
 <tile id="82">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="84">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="5"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="84" duration="100"/>
   <frame tileid="85" duration="100"/>
   <frame tileid="86" duration="100"/>
   <frame tileid="87" duration="100"/>
  </animation>
 </tile>
 <tile id="86">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="88">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="6"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="88" duration="100"/>
   <frame tileid="89" duration="100"/>
   <frame tileid="90" duration="100"/>
   <frame tileid="91" duration="100"/>
  </animation>
 </tile>
 <tile id="90">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="92">
  <properties>
   <property name="action" value="run"/>
   <property name="dir" type="int" value="7"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="92" duration="100"/>
   <frame tileid="93" duration="100"/>
   <frame tileid="94" duration="100"/>
   <frame tileid="95" duration="100"/>
  </animation>
 </tile>
 <tile id="94">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="96">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="0"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="96" duration="150"/>
   <frame tileid="97" duration="150"/>
   <frame tileid="98" duration="150"/>
   <frame tileid="99" duration="150"/>
  </animation>
 </tile>
 <tile id="98">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="100">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="1"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="100" duration="150"/>
   <frame tileid="101" duration="150"/>
   <frame tileid="102" duration="150"/>
   <frame tileid="103" duration="150"/>
  </animation>
 </tile>
 <tile id="102">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="104">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="2"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="104" duration="150"/>
   <frame tileid="105" duration="150"/>
   <frame tileid="106" duration="150"/>
   <frame tileid="107" duration="150"/>
  </animation>
 </tile>
 <tile id="106">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="108">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="3"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="108" duration="150"/>
   <frame tileid="109" duration="150"/>
   <frame tileid="110" duration="150"/>
   <frame tileid="111" duration="150"/>
  </animation>
 </tile>
 <tile id="110">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="112">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="4"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="112" duration="150"/>
   <frame tileid="113" duration="150"/>
   <frame tileid="114" duration="150"/>
   <frame tileid="115" duration="150"/>
  </animation>
 </tile>
 <tile id="114">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="116">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="5"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="116" duration="150"/>
   <frame tileid="117" duration="150"/>
   <frame tileid="118" duration="150"/>
   <frame tileid="119" duration="150"/>
  </animation>
 </tile>
 <tile id="118">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="120">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="6"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="120" duration="150"/>
   <frame tileid="121" duration="150"/>
   <frame tileid="122" duration="150"/>
   <frame tileid="123" duration="150"/>
  </animation>
 </tile>
 <tile id="122">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
 <tile id="124">
  <properties>
   <property name="action" value="walk"/>
   <property name="dir" type="int" value="7"/>
   <property name="step" type="bool" value="true"/>
  </properties>
  <animation>
   <frame tileid="124" duration="150"/>
   <frame tileid="125" duration="150"/>
   <frame tileid="126" duration="150"/>
   <frame tileid="127" duration="150"/>
  </animation>
 </tile>
 <tile id="126">
  <properties>
   <property name="step" type="bool" value="true"/>
  </properties>
 </tile>
</tileset>
